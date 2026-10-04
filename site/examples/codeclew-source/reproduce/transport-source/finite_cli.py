"""Private finite CLI primitives adapted from the accepted reviewer candidate.
No provider calls occur on import. Fake CLI requires explicit offline settings.
"""
from __future__ import annotations
from datetime import datetime, timezone
from pathlib import Path
from dataclasses import dataclass
import hashlib, json, math, os, re, selectors, signal, socket, subprocess, threading, time
CODEX_EXECUTABLE = "/opt/homebrew/bin/codex"
EXPECTED_MODEL = "gpt-6.1-sol"
RESULT_SCHEMA = "codeclew-documentation-agent-result/1.0"
DEADLINE_MARGIN_SECONDS = 3.0
POLL_SECONDS = 0.05
MAX_STDOUT_BYTES = 4 * 1024 * 1024
MAX_STDERR_BYTES = 1024 * 1024
MAX_EVENTS = 10000
SHUTDOWN = threading.Event()
ALLOWED_EVENTS = frozenset({'thread.started','turn.started','turn.completed','turn.failed','item.started','item.updated','item.completed','error'})
SAFE_ID = re.compile(r'^[A-Za-z0-9_-]{1,128}$')
class JobError(ValueError):
    pass


class InvocationError(RuntimeError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class RunnerSettings:
    offline_fixture: bool = False
    fake_codex_executable: str | None = None

    def executable(self) -> str:
        if self.offline_fixture:
            if not self.fake_codex_executable:
                raise ValueError("offline fixture requires an explicit fake CLI")
            return self.fake_codex_executable
        if self.fake_codex_executable is not None:
            raise ValueError("fake CLI paths are available only to offline fixtures")
        return CODEX_EXECUTABLE


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace(
        "+00:00", "Z"
    )


def _reject_constant(value: str):
    raise JobError("non-standard JSON number: " + value)


def _unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise JobError("duplicate JSON object key")
        result[key] = value
    return result


def _json_loads(raw: bytes):
    try:
        return json.loads(
            raw.decode("utf-8", errors="strict"),
            object_pairs_hook=_unique_object,
            parse_constant=_reject_constant,
        )
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise JobError("request is not valid UTF-8 JSON") from error


def _canonical_json_bytes(value) -> bytes:
    try:
        return json.dumps(
            value, ensure_ascii=False, sort_keys=True, separators=(",", ":")
        ).encode("utf-8", errors="strict")
    except (TypeError, UnicodeEncodeError) as error:
        raise JobError("output schema is not valid JSON data") from error


def digest(value):
    return "sha256:" + hashlib.sha256(_canonical_json_bytes(value)).hexdigest()


def parse_bridge_deadline(value: str | None) -> float | None:
    if value is None:
        return None
    try:
        deadline_at = float(value)
    except ValueError as error:
        raise JobError("bridge deadline is invalid") from error
    if not math.isfinite(deadline_at) or deadline_at <= 0:
        raise JobError("bridge deadline is invalid")
    return deadline_at


def effective_deadline_at(
    request_started: float, job: dict, bridge_deadline_at: float | None
) -> float:
    job_deadline_at = (
        request_started
        + job["cap"]["timeoutMs"] / 1000.0
        - DEADLINE_MARGIN_SECONDS
    )
    if bridge_deadline_at is None:
        return job_deadline_at
    return min(job_deadline_at, bridge_deadline_at)


def build_command(
    executable: str, scratch: Path, output_path: Path, schema_path: Path
) -> list[str]:
    return [
        executable,
        "-a", "never", "exec",
        "--strict-config",
        "--ignore-user-config",
        "--ignore-rules",
        "--skip-git-repo-check",
        "--json",
        "--color", "never",
        "--sandbox", "read-only",
        "-c", "features.shell_tool=false",
        "-c", "features.unified_exec=false",
        "-c", "features.multi_agent=false",
        "-c", "features.apps=false",
        "-c", "features.plugins=false",
        "-c", 'web_search="disabled"',
        "-c", "project_doc_max_bytes=0",
        "-c", "skills.include_instructions=false",
        "-c", "skills.bundled.enabled=false",
        "-c", "include_apps_instructions=false",
        "-c", "include_collaboration_mode_instructions=false",
        "-c", 'model_reasoning_effort="high"',
        "--model", EXPECTED_MODEL,
        "--cd", str(scratch),
        "--output-schema", str(schema_path.resolve(strict=False)),
        "--ephemeral",
        "-",
    ]


def _write_once(path: Path, data: bytes, mode: int = 0o600) -> None:
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, mode)
    try:
        with os.fdopen(descriptor, "wb", closefd=False) as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
    finally:
        os.close(descriptor)


def _write_json_once(path: Path, value) -> None:
    data = (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n")
    _write_once(path, data.encode("utf-8"))


def _raw_json_value(raw):
    text = raw.decode('utf-8', errors='strict')
    decoder = json.JSONDecoder(object_pairs_hook=_unique_object,
                               parse_constant=_reject_constant)
    start = len(text) - len(text.lstrip())
    value, end = decoder.raw_decode(text, start)
    if text[end:].strip():
        raise JobError('final message contains trailing non-JSON content')
    return value, text[start:end].encode('utf-8')


class StreamAccounting:
    def __init__(self, output_cap):
        self.output_cap = output_cap
        self.stdout_bytes = self.stderr_bytes = self.events = 0
        self.buffer = bytearray()
        self.stderr_buffer = bytearray()
        self.counts = {}
        self.thread_id = None
        self.turn_id = None
        self.started = self.completed = self.messages = 0
        self.ambiguous = False
        self.tokens = None
        self.final = None
        self.usage_reason = 'NO_COMPLETED_TURN'

    def feed(self, stream, chunk):
        if stream == 'stderr':
            # Only CLI runtime/error channel. Never collect stdout reasoning.
            room = MAX_STDERR_BYTES - len(self.stderr_buffer)
            self.stderr_buffer.extend(chunk[:room])
            self.stderr_bytes += len(chunk)
            if self.stderr_bytes > MAX_STDERR_BYTES:
                raise InvocationError('STDERR_LIMIT', 'CLI stderr byte limit exceeded')
            return
        self.stdout_bytes += len(chunk)
        if self.stdout_bytes > MAX_STDOUT_BYTES:
            raise InvocationError('STDOUT_LIMIT', 'CLI stdout byte limit exceeded')
        self.buffer.extend(chunk)
        while b'\n' in self.buffer:
            offset = self.buffer.index(b'\n')
            if offset > self.output_cap * 6 + 65536:
                raise InvocationError('EVENT_LINE_LIMIT', 'CLI JSONL line limit exceeded')
            line = bytes(self.buffer[:offset])
            del self.buffer[:offset + 1]
            self.event(line)
        if len(self.buffer) > self.output_cap * 6 + 65536:
            raise InvocationError('EVENT_LINE_LIMIT', 'CLI JSONL line limit exceeded')

    def event(self, line):
        self.events += 1
        if self.events > MAX_EVENTS:
            raise InvocationError('EVENT_COUNT_LIMIT', 'CLI event count limit exceeded')
        try:
            event = _json_loads(line)
        except (JobError, ValueError, RecursionError):
            self.ambiguous = True
            self.usage_reason = 'MALFORMED_STREAM'
            return
        if not isinstance(event, dict):
            self.ambiguous = True
            return
        kind = event.get('type')
        if not isinstance(kind, str):
            self.ambiguous = True
            return
        key = kind if kind in ALLOWED_EVENTS else 'unknown'
        self.counts[key] = self.counts.get(key, 0) + 1
        if key == 'unknown':
            self.ambiguous = True
        # Optional emitted identities are authority only when they agree with
        # this fresh process's already established thread/turn.
        if kind != 'thread.started' and 'thread_id' in event:
            identifier = event['thread_id']
            if not isinstance(identifier, str) or identifier != self.thread_id:
                self.ambiguous = True
        if kind != 'turn.started' and 'turn_id' in event:
            identifier = event['turn_id']
            if not isinstance(identifier, str) or self.turn_id is None or identifier != self.turn_id:
                self.ambiguous = True
        if kind == 'thread.started':
            identifier = event.get('thread_id')
            if self.thread_id is not None or self.started or not isinstance(identifier, str) or not SAFE_ID.fullmatch(identifier):
                self.ambiguous = True
            else:
                self.thread_id = identifier
        elif kind == 'turn.started':
            self.started += 1
            if self.started != 1 or self.thread_id is None or self.completed:
                self.ambiguous = True
            identifier = event.get('turn_id')
            if 'turn_id' in event:
                if not isinstance(identifier, str) or not SAFE_ID.fullmatch(identifier):
                    self.ambiguous = True
                else:
                    self.turn_id = identifier
        elif kind.startswith('item.'):
            item = event.get('item')
            if not isinstance(item, dict):
                self.ambiguous = True
                return
            item_type = item.get('type')
            if item_type not in ('reasoning', 'agent_message'):
                raise InvocationError('UNEXPECTED_TOOL_EVENT', 'disabled or unknown reviewer item observed')
            if kind == 'item.completed' and item_type == 'agent_message':
                self.messages += 1
                if self.started != 1 or self.completed or self.messages != 1:
                    self.ambiguous = True
                text = item.get('text')
                if not isinstance(text, str):
                    raise InvocationError('FINAL_MESSAGE_MISSING', 'agent message text missing')
                final = text.encode('utf-8', errors='strict')
                if len(final) > self.output_cap:
                    raise InvocationError('FINAL_MESSAGE_LIMIT', 'final answer byte limit exceeded')
                self.final = final
            # Reasoning payloads are discarded; no content, ID, digest or summary retained.
        elif kind == 'turn.completed':
            self.completed += 1
            if self.started != 1 or self.completed != 1 or self.messages != 1:
                self.ambiguous = True
            identifier = event.get('turn_id')
            if identifier != self.turn_id:
                self.ambiguous = True
            usage = event.get('usage')
            required = ('input_tokens', 'output_tokens', 'cached_input_tokens')
            optional = ('reasoning_output_tokens', 'total_tokens')
            if (not isinstance(usage, dict) or
                any(type(usage.get(k)) is not int or not 0 <= usage[k] <= 2**63-1 for k in required) or
                any(k in usage and (type(usage[k]) is not int or not 0 <= usage[k] <= 2**63-1) for k in optional)):
                self.usage_reason = 'MISSING_OR_MALFORMED_USAGE'
                return
            if (usage['cached_input_tokens'] > usage['input_tokens'] or
                usage.get('reasoning_output_tokens', 0) > usage['output_tokens'] or
                ('total_tokens' in usage and usage['total_tokens'] != usage['input_tokens'] + usage['output_tokens'])):
                self.usage_reason = 'INCONSISTENT_USAGE'
                return
            self.tokens = {k: usage[k] for k in required + optional if k in usage}
            self.usage_reason = 'OBSERVED_SINGLE_COMPLETED_CLI_TURN'
        elif kind in ('turn.failed', 'error'):
            self.ambiguous = True
            self.usage_reason = 'CLI_FAILURE_EVENT'

    def metadata(self, successful=False):
        valid = successful and not self.ambiguous and self.thread_id is not None and self.started == self.completed == self.messages == 1
        return {'stdoutBytes': self.stdout_bytes, 'stderrBytes': self.stderr_bytes,
                'eventCount': self.events, 'eventCounts': self.counts,
                'threadId': self.thread_id, 'turnId': self.turn_id,
                'localTurnOrdinal': 1 if valid else None,
                'tokenUsage': self.tokens if valid else None,
                'tokenUsageAuthority': 'CLI_REPORTED' if valid and self.tokens is not None else 'UNKNOWN',
                'tokenUsageReason': self.usage_reason if valid else 'INCOMPLETE_OR_AMBIGUOUS_STREAM',
                'costUsage': None, 'costUsageAuthority': 'UNKNOWN',
                'usage': None, 'usageAuthority': 'MAXIMUM_ONLY',
                'providerCancellation': 'NOT_ESTABLISHED'}


def _stop_process(process):
    # The fresh CLI session owns one local group. Kill even after its leader exits
    # so descendants cannot outlive a successful or failed transport invocation.
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    try:
        return process.wait(timeout=1.0)
    except subprocess.TimeoutExpired:
        raise InvocationError('LOCAL_CLEANUP_TIMEOUT', 'local process group cleanup did not complete') from None


def peer_gone(connection):
    try:
        readable, _, _ = __import__('select').select([connection], [], [], 0)
        return bool(readable) and connection.recv(1, socket.MSG_PEEK | socket.MSG_DONTWAIT) == b''
    except (OSError, ValueError):
        return True


def _output_validation_failure(error):
    # Only fixed local validator diagnostics become metadata. Never include the
    # exception itself, output text, source text, paths or arbitrary key names.
    message = str(error)
    bounds = {
        'output nesting exceeds bound', 'too many properties', 'array bounds',
        'string bounds', 'summary operation bound', 'native summary prose byte bound',
        'title byte bound', 'summary uncertainty byte bound', 'uncertainty/gap byte bound',
        'review text byte bound',
    }
    schema = {
        'value prohibited by native schema', 'union output does not match',
        'constant mismatch', 'enum mismatch', 'output type mismatch',
        'missing required output fields', 'unknown output field', 'duplicate coverage',
        'string pattern', 'duplicate or conflicting summary target',
        'sequence steps outside bounded summary contract',
        'verified section summaries/gaps not covered', 'approval contradicts errors',
        'nonapproval lacks issue',
    }
    if message in bounds:
        return 'FINAL_OUTPUT_BOUND_INVALID', 'BOUNDS'
    if message in schema:
        return 'FINAL_SCHEMA_INVALID', 'SCHEMA'
    return 'FINAL_VALIDATION_REFUSED', 'VALIDATION'


def run_model(raw, job, directory, request_started, request_started_utc,
               settings=None, bridge_deadline_at=None, cancelled=None):
    settings = PRODUCTION_SETTINGS if settings is None else settings
    deadline = effective_deadline_at(request_started, job, bridge_deadline_at)
    directory.mkdir(mode=0o700, parents=False, exist_ok=False)
    _write_once(directory / 'request.json', raw)
    prompt = build_prompt(job['payload'])
    _write_once(directory / 'prompt.txt', prompt)
    scratch = directory / 'empty-scratch'
    scratch.mkdir(mode=0o700)
    schema = directory / 'output-schema.json'
    _write_json_once(schema, job['_projected_output_schema'])
    _write_json_once(directory / 'schema-projection.json', job['_schema_projection_metadata'])
    command = build_command(settings.executable(), scratch, directory / 'unused-output', schema)
    _write_json_once(directory / 'argv.json', command)
    accounting = StreamAccounting(job['cap']['outputBytes'])
    process = None
    failure = None
    exit_code = None
    failure_stage = 'CLI_EXECUTION'
    validation_failure = None
    unaccepted_final_retained = False
    try:
        if time.monotonic() >= deadline:
            raise InvocationError('TIMEOUT_BEFORE_LAUNCH', 'job deadline expired')
        process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, cwd=scratch,
                                   start_new_session=True, close_fds=True)
        _write_json_once(directory / 'launched.json', {'pid': process.pid, 'processGroup': process.pid, 'startedUtc': utc_now()})
        with selectors.DefaultSelector() as selector:
            for stream, name in [(process.stdin, 'stdin'), (process.stdout, 'stdout'), (process.stderr, 'stderr')]:
                os.set_blocking(stream.fileno(), False)
                selector.register(stream, selectors.EVENT_WRITE if name == 'stdin' else selectors.EVENT_READ, name)
            sent = 0
            while selector.get_map():
                if SHUTDOWN.is_set() or (cancelled is not None and cancelled()):
                    raise InvocationError('LOCAL_CANCELLED', 'bridge or controller stopped locally')
                if time.monotonic() >= deadline:
                    raise InvocationError('REVIEWER_TIMEOUT', 'local reviewer deadline expired')
                for key, _mask in selector.select(POLL_SECONDS):
                    stream, name = key.fileobj, key.data
                    if name == 'stdin':
                        try:
                            sent += os.write(stream.fileno(), prompt[sent:sent + 8192])
                        except BrokenPipeError:
                            raise InvocationError('INCOMPLETE_PROMPT_DELIVERY', 'CLI closed input before complete payload')
                        if sent == len(prompt):
                            selector.unregister(stream)
                            stream.close()
                    else:
                        chunk = os.read(stream.fileno(), 8192)
                        if not chunk:
                            selector.unregister(stream)
                            stream.close()
                        else:
                            accounting.feed(name, chunk)
                # A finished leader cannot delegate indefinite pipe ownership.
                if process.poll() is not None:
                    _stop_process(process)
                # Child descendants holding pipes remain deadline/cancellation bounded.
            if accounting.buffer:
                accounting.event(bytes(accounting.buffer))
                accounting.buffer.clear()
            exit_code = process.wait(timeout=max(.001, deadline - time.monotonic()))
        if exit_code != 0:
            raise InvocationError('CODEX_EXIT_NONZERO', 'CLI failed')
        if accounting.final is None:
            raise InvocationError('FINAL_MESSAGE_MISSING', 'no final agent message')
        failure_stage = 'SAVE_UNACCEPTED_FINAL'
        # StreamAccounting already enforces the native output byte cap. Recheck
        # before writing: this private file is diagnostic, never native admission.
        if len(accounting.final) > job['cap']['outputBytes']:
            raise InvocationError('FINAL_MESSAGE_LIMIT', 'final answer byte limit exceeded')
        _write_once(directory / 'unaccepted-final-message.raw', accounting.final)
        unaccepted_final_retained = True
        failure_stage = 'FINAL_IDENTITY'
        if accounting.ambiguous or accounting.started != 1 or accounting.completed != 1 or accounting.messages != 1:
            raise InvocationError('AMBIGUOUS_CLI_RESULT', 'not exactly one unambiguous fresh completed turn')
        failure_stage = 'FINAL_JSON'
        try:
            value, exact = _raw_json_value(accounting.final)
        except (ValueError, UnicodeError, RecursionError) as error:
            validation_failure = {'stage': failure_stage, 'code': 'FINAL_MESSAGE_NOT_JSON', 'category': 'JSON'}
            raise InvocationError('FINAL_MESSAGE_NOT_JSON', 'final message is not one exact JSON value') from error
        failure_stage = 'OUTPUT_VALIDATION'
        try:
            validate_output(value, job)
        except JobError as error:
            code, category = _output_validation_failure(error)
            validation_failure = {'stage': failure_stage, 'code': code, 'category': category}
            raise InvocationError(code, 'final output validation refused') from error
        except (ValueError, TypeError, KeyError, RecursionError) as error:
            validation_failure = {'stage': failure_stage, 'code': 'FINAL_VALIDATION_EXCEPTION', 'category': 'VALIDATION'}
            raise InvocationError('FINAL_VALIDATION_EXCEPTION', 'final output validator failed closed') from error
        failure_stage = 'RESPONSE_BUILD'
        envelope = json.dumps({'schema': RESULT_SCHEMA, 'invocation': job['invocation'],
                               'role': job['role'], 'model': EXPECTED_MODEL, 'usage': None}, separators=(',', ':')).encode()
        response = envelope[:-1] + b',"result":' + exact + b'}'
        if len(response) > job['cap']['outputBytes']:
            validation_failure = {'stage': failure_stage, 'code': 'RESPONSE_EXCEEDS_CAP', 'category': 'BOUNDS'}
            raise InvocationError('RESPONSE_EXCEEDS_CAP', 'full native response exceeds output cap')
        failure_stage = 'SAVE_ACCEPTED_RESULT'
        _write_once(directory / 'final-message.raw', accounting.final)
        _write_once(directory / 'response.json', response)
        return response
    except InvocationError as error:
        failure = error.code
        raise
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        failure = 'LOCAL_TRANSPORT_ERROR'
        raise InvocationError(failure, 'local transport failed') from error
    finally:
        if process is not None:
            exit_code = _stop_process(process)
            for stream in (process.stdin, process.stdout, process.stderr):
                if stream is not None and not stream.closed:
                    stream.close()
        stderr_retention = 'NOT_REQUIRED'
        if failure_stage == 'CLI_EXECUTION' and failure in {'CODEX_EXIT_NONZERO', 'LOCAL_TRANSPORT_ERROR', 'STDERR_LIMIT'}:
            try:
                _write_once(directory / 'cli-stderr.raw', bytes(accounting.stderr_buffer))
                stderr_retention = 'PRIVATE_BOUNDED_CAPTURE'
            except OSError:
                # No arbitrary exception/path/error prose is copied to metadata.
                stderr_retention = 'WRITE_FAILED'
        meta = accounting.metadata(successful=failure is None and exit_code == 0)
        meta.update({'invocation': job['invocation'], 'model': EXPECTED_MODEL,
                     'cliExitCode': exit_code, 'failure': failure,
                     'cliStderrRetention': stderr_retention,
                     'cliStderrFile': 'cli-stderr.raw' if stderr_retention == 'PRIVATE_BOUNDED_CAPTURE' else None,
                     'cliStderrBytesRetained': len(accounting.stderr_buffer) if stderr_retention == 'PRIVATE_BOUNDED_CAPTURE' else 0,
                     'cliStderrTruncated': len(accounting.stderr_buffer) < accounting.stderr_bytes if stderr_retention == 'PRIVATE_BOUNDED_CAPTURE' else None,
                     'failureStage': failure_stage if failure else None,
                     'validationFailure': validation_failure,
                     'unacceptedFinalRetained': unaccepted_final_retained,
                     'unacceptedFinalFile': 'unaccepted-final-message.raw' if unaccepted_final_retained else None,
                     'elapsedSeconds': round(time.monotonic() - request_started, 6),
                     'cap': job['cap'], 'requestBytes': len(raw),
                     'promptBytes': len(prompt), 'promptSha256': hashlib.sha256(prompt).hexdigest(),
                     'promptInputBound': 'UTF8_BYTES_ONLY_NOT_PROVIDER_TOTAL',
                     'localPollingSeconds': POLL_SECONDS,
                     'streamLimits': {'stdoutBytes': MAX_STDOUT_BYTES, 'stderrBytes': MAX_STDERR_BYTES, 'events': MAX_EVENTS},
                     'sourceOfFinalMessage': 'BOUNDED_CLI_AGENT_MESSAGE',
                     'providerCostOrTokenCapEnforced': False})
        _write_json_once(directory / ('failure.json' if failure else 'completed.json'), meta)


PRODUCTION_SETTINGS = RunnerSettings()
