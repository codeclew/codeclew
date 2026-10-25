#!/usr/bin/env python3
"""Owned protocol and failure tests for the read-only proposal collector."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
HELPER = ROOT / "skills/codeclew/scripts/collect_proposal.py"
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("collect_proposal", HELPER)
helper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helper)


def page(items=(), cursor=None, total=None, **fields):
    return {"schema": "codeclew-docs-page/1.0", "reportSchema": "codeclew-documentation-proposal-result/1.0",
            "proposal": "proposal-1", "work": "work-1", "status": "READY_WITH_LIMITATIONS",
            "meaningReview": "UNASSESSED", "readDigest": "sha256:reads", "narrativeDigest": "sha256:body",
            "items": list(items), "omitted": [], "total": len(items) if total is None else total,
            "nextCursor": cursor, **fields}


def row(kind="CLAIM", identifier="claim-1", record=None):
    return {"kind": kind, "id": identifier, "record": {"text": "Private retained text"} if record is None else record}


def work_page(items=(), cursor=None, total=None, **fields):
    return {"schema":"codeclew-documentation-work-page/1.0", "work":"work-1",
            "snapshot":"sha256:snapshot/123", "subject":"service:sample", "audience":"Maintainers",
            "contextDigest":"sha256:context", "inputDigest":"sha256:inputs", "membershipDigest":"sha256:members",
            "receiptDigest":"sha256:receipt", "authority":"IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
            "influenceCoverage":"RECORDED_READS_ONLY_EXECUTION_NOT_ATTESTED",
            "items":list(items), "omitted":[], "total":len(items) if total is None else total,
            "nextCursor":cursor, **fields}


class CollectorTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.root = self.base / "docs-root"
        self.root.mkdir()
        (self.root / "unchanged.md").write_bytes(b"Human text\r\n")
        self.launcher = self.base / "clew"
        self.output = self.base / "collection"

    def driver(self, responses):
        fixture = self.base / "responses.json"
        fixture.write_text(json.dumps(responses))
        self.launcher.write_text(f"#!{sys.executable}\n" +
            "import json,sys,time\nfrom pathlib import Path\n" +
            f"fixture=Path({str(fixture)!r})\n" +
            "args=sys.argv[1:]\n" +
            "if args[:3]==['docs','proposal','show']:\n" +
            " assert set(args[3::2]) <= {'--root','--proposal','--limit','--cursor'}\n" +
            " cursor=args[args.index('--cursor')+1] if '--cursor' in args else 'FIRST'\n" +
            "else:\n" +
            " assert args[:3]==['docs','work','read']\n" +
            " assert set(args[3::2])=={'--root','--work','--input'}\n" +
            " selection=json.loads(Path(args[args.index('--input')+1]).read_text())\n" +
            " assert set(selection)<= {'cursor','references'}\n" +
            " cursor=selection.get('cursor','FIRST')\n" +
            "response=json.loads(fixture.read_text())[cursor]\n" +
            "time.sleep(response.get('sleep',0))\n" +
            "sys.stdout.write(response.get('raw',json.dumps(response.get('page'))))\n" +
            "sys.stderr.write(response.get('stderr',''))\n" +
            "sys.exit(response.get('exit',0))\n")
        self.launcher.chmod(0o700)

    def collect(self, responses, **options):
        before = {p.name: p.read_bytes() for p in self.root.iterdir()}
        self.driver(responses)
        result = helper.collect(self.launcher, self.root, "proposal-1", self.output, **options)
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.root.iterdir()})
        self.assertEqual(result, json.loads((self.output / "manifest.json").read_text()))
        return result

    def test_complete_pages_preserve_order_raw_identity_and_limitations(self):
        first = page([row()], "next", 2)
        second = page([row("DIAGNOSTIC", "diagnostic-1", {"code": "UNRESOLVED", "detail": "private diagnostic"})], total=2)
        result = self.collect({"FIRST": {"page": first}, "next": {"page": second}})
        self.assertEqual(result["status"], "COMPLETE")
        self.assertEqual(result["nativeStatus"], "READY_WITH_LIMITATIONS")
        self.assertEqual(result["meaningReview"], "UNASSESSED")
        self.assertTrue(result["terminalCursorObserved"])
        self.assertEqual(result["diagnosticCodes"], {"UNRESOLVED": 1})
        self.assertEqual(json.loads((self.output / "items.json").read_text()), first["items"] + second["items"])
        self.assertEqual(json.loads((self.output / "page-0001.stdout").read_text()), first)
        self.assertEqual(result["publication"], "NOT_PERFORMED")

    def test_empty_nonterminal_page_requires_continuation(self):
        result = self.collect({"FIRST": {"page": page([], "next", 1)}, "next": {"page": page([row()], total=1)}})
        self.assertEqual(result["status"], "COMPLETE")
        self.assertEqual(result["pages"], 2)

    def test_empty_terminal_is_valid_only_with_zero_total(self):
        result = self.collect({"FIRST": {"page": page()}})
        self.assertEqual(result["status"], "COMPLETE")
        self.assertEqual(result["rows"], 0)

    def test_needs_repair_delivery_is_not_relabelled_ready_or_reviewed(self):
        result = self.collect({"FIRST": {"page": page([row("DIAGNOSTIC", "d", {"code": "FLOW_NOT_DOCUMENTED"})], status="NEEDS_REPAIR", narrativeDigest=None)}})
        self.assertEqual(result["status"], "COMPLETE")
        self.assertEqual(result["nativeStatus"], "NEEDS_REPAIR")
        self.assertEqual(result["meaningValidation"], "NOT_PERFORMED")

    def test_every_bound_identity_change_fails_with_both_raw_pages_preserved(self):
        for field in ("proposal", "work", "status", "meaningReview", "readDigest", "narrativeDigest", "total", "reportSchema"):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as directory:
                self.output = Path(directory) / "collection"
                second = page([row(identifier="c2")], total=2)
                second[field] = 3 if field == "total" else "changed"
                result = self.collect({"FIRST": {"page": page([row()], "next", 2)}, "next": {"page": second}})
                self.assertEqual(result["status"], "INCOMPLETE")
                self.assertTrue((self.output / "page-0002.stdout").is_file())
                self.assertEqual(result["rows"], 1)

    def test_missing_terminal_schema_items_and_invalid_json_fail_closed(self):
        variants = []
        for key in ("nextCursor", "omitted", "items", "work", "readDigest"):
            bad = page([row()]); del bad[key]
            variants.append({"page": bad})
        variants += [{"page": {"error": {"message": "native failure"}}}, {"raw": "{broken"},
                     {"raw": '{"duplicate":1,"duplicate":2}'}, {"raw": '{"value":NaN}'},
                     {"page": []}, {"page": page([None])}]
        for response in variants:
            with self.subTest(response=response), tempfile.TemporaryDirectory() as directory:
                self.output = Path(directory) / "collection"
                result = self.collect({"FIRST": response})
                self.assertEqual(result["status"], "INCOMPLETE")
                self.assertFalse(result["terminalCursorObserved"])
                self.assertTrue((self.output / "page-0001.stdout").is_file())

    def test_omitted_records_are_saved_and_never_reported_complete(self):
        result = self.collect({"FIRST": {"page": page([], total=1, omitted=[{"id": "large", "reason": "ITEM_EXCEEDS_STDOUT_BUDGET"}])}})
        self.assertEqual(result["reason"], "OMITTED_RECORDS_REQUIRE_NATIVE_INSPECTION")
        self.assertTrue(result["terminalCursorObserved"])
        self.assertEqual(len(json.loads((self.output / "omitted.json").read_text())), 1)

    def test_repeated_cursor_and_repeated_items_stop_without_retry(self):
        result = self.collect({"FIRST": {"page": page([], "same", 1)}, "same": {"page": page([], "same", 1)}})
        self.assertEqual(result["reason"], "REPEATED_CURSOR")
        self.assertEqual(result["pages"], 2)
        self.output = self.base / "duplicates"
        result = self.collect({"FIRST": {"page": page([row()], "next", 2)}, "next": {"page": page([row()], total=2)}})
        self.assertEqual(result["reason"], "REPEATED_ITEM")

    def test_limits_and_row_count_cannot_silently_truncate(self):
        cases = [({"FIRST": {"page": page([], "next", 1)}}, {"max_pages": 1}, "HELPER_PAGE_LIMIT_BEFORE_TERMINAL"),
                 ({"FIRST": {"page": page([row()], total=2)}}, {}, "TOTAL_COUNT_MISMATCH"),
                 ({"FIRST": {"page": page([row()])}}, {"max_page_bytes": 20}, "HELPER_PAGE_BYTE_LIMIT")]
        for responses, options, reason in cases:
            with self.subTest(reason=reason), tempfile.TemporaryDirectory() as directory:
                self.output = Path(directory) / "collection"
                self.assertEqual(self.collect(responses, **options)["reason"], reason)

    def test_process_failure_and_timeout_preserve_raw_output_and_do_not_retry(self):
        result = self.collect({"FIRST": {"exit": 2, "raw": "failed response", "stderr": "native error"}})
        self.assertEqual(result["reason"], "NATIVE_PROCESS_FAILED")
        self.assertEqual((self.output / "page-0001.stderr").read_text(), "native error")
        self.output = self.base / "timeout"
        result = self.collect({"FIRST": {"sleep": 1}}, timeout=0.05)
        self.assertEqual(result["reason"], "PROCESS_TIMEOUT_NO_AUTOMATIC_RETRY")
        self.assertEqual(result["pages"], 1)

    def test_missing_launcher_and_existing_output_do_not_overwrite(self):
        result = helper.collect(self.base / "missing", self.root, "proposal-1", self.output)
        self.assertEqual(result["status"], "INCOMPLETE")
        frozen = (self.output / "manifest.json").read_bytes()
        with self.assertRaises(FileExistsError):
            helper.collect(self.base / "missing", self.root, "proposal-1", self.output)
        self.assertEqual((self.output / "manifest.json").read_bytes(), frozen)

    def test_cli_summary_keeps_business_text_in_files(self):
        self.driver({"FIRST": {"page": page([row()])}})
        process = subprocess.run([sys.executable, "-I", "-S", str(HELPER), "--launcher", str(self.launcher),
                                  "--root", str(self.root), "--proposal", "proposal-1", "--output", str(self.output)], capture_output=True, text=True)
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertNotIn("Private retained text", process.stdout)
        self.assertEqual(json.loads(process.stdout)["nativeStatus"], "READY_WITH_LIMITATIONS")

    def test_work_initial_uses_only_default_then_cursor_preserving_native_receipts(self):
        self.driver({"FIRST":{"page":work_page([row("SOURCE", "s1")],"next",2)},
                     "next":{"page":work_page([row("DEPENDENCY", "d1")],total=2,receiptDigest="sha256:second")}})
        result=helper.collect(self.launcher,self.root,"work-1",self.output,kind="work-initial")
        self.assertEqual(result["status"],"COMPLETE")
        self.assertEqual(result["readScope"],"INITIAL_DEFAULT_ONLY")
        self.assertEqual(result["identity"]["membershipDigest"],"sha256:members")
        self.assertNotIn("receiptDigest",result["identity"])
        self.assertIsNone(result["nativeStatus"])
        self.assertIsNone(result["meaningReview"])
        self.assertEqual(json.loads((self.output/"page-0001.selection.json").read_text()),{})
        self.assertEqual(json.loads((self.output/"page-0002.selection.json").read_text()),{"cursor":"next"})
        for page in result["artifacts"]["pages"]:
            self.assertEqual(page["command"][1:4],["docs","work","read"])
            self.assertNotIn("--limit",page["command"])
        self.assertEqual(json.loads((self.output/"page-0002.stdout").read_text())["receiptDigest"],"sha256:second")
        self.assertEqual((self.root/"unchanged.md").read_bytes(),b"Human text\r\n")

    def test_work_initial_identity_and_future_authority_drift_fail_closed(self):
        for field in ("work","snapshot","subject","contextDigest","inputDigest","membershipDigest","authority","futureEvidenceAuthority"):
            with self.subTest(field=field),tempfile.TemporaryDirectory() as directory:
                self.output=Path(directory)/"collection"
                first=work_page([row("SOURCE", "s1")],"next",2,futureEvidenceAuthority="old")
                second=work_page([row("SOURCE", "s2")],total=2,futureEvidenceAuthority="old");second[field]="changed"
                self.driver({"FIRST":{"page":first},"next":{"page":second}})
                result=helper.collect(self.launcher,self.root,"work-1",self.output,kind="work-initial")
                self.assertEqual(result["status"],"INCOMPLETE")
                self.assertEqual(result["pages"],2)
                self.assertEqual(result["rows"],1)
                self.assertTrue((self.output/"page-0002.stdout").is_file())

    def test_work_initial_omissions_untracked_reads_and_missing_receipt_remain_incomplete(self):
        variants=[work_page([],total=1,omitted=[{"kind":"SOURCE","id":"large","recovery":{"reference":"s1"}}]),
                  work_page(influenceCoverage="INCOMPLETE_UNTRACKED_READS")]
        missing=work_page();del missing["receiptDigest"];variants.append(missing)
        for page in variants:
            with self.subTest(page=page),tempfile.TemporaryDirectory() as directory:
                self.output=Path(directory)/"collection";self.driver({"FIRST":{"page":page}})
                result=helper.collect(self.launcher,self.root,"work-1",self.output,kind="work-initial")
                self.assertEqual(result["status"],"INCOMPLETE")
                self.assertEqual(result["pages"],1)

    def test_work_initial_cli_wrapper_is_isolated_and_never_adds_a_selector(self):
        self.driver({"FIRST":{"page":work_page([],"next",1)},"next":{"page":work_page([row("SOURCE","s1")])}})
        process=subprocess.run([sys.executable,"-I","-S",str(HELPER.with_name("collect_work_initial.py")),"--launcher",str(self.launcher),"--root",str(self.root),"--work","work-1","--output",str(self.output)],capture_output=True,text=True)
        self.assertEqual(process.returncode,0,process.stderr)
        self.assertEqual(json.loads(process.stdout)["status"],"COMPLETE")
        self.assertEqual(json.loads((self.output/"page-0002.selection.json").read_text()),{"cursor":"next"})

    def test_work_initial_preserves_flat_coverage_and_mixed_rows_verbatim(self):
        coverage={"kind":"COVERAGE","id":"sample","revision":"owned-revision",
                  "extractor":"SOURCE_SYNTAX","runtimeMode":"RELEASE","coverage":"PARTIAL",
                  "boundaries":["CALL_TARGETS_UNRESOLVED"],"callAuthority":"SYNTAX_UNRESOLVED",
                  "referenceRoles":[]}
        records=[coverage,row("ENTRYPOINT","entry"),row("SOURCE","source")]
        self.driver({"FIRST":{"page":work_page(records)}})
        result=helper.collect(self.launcher,self.root,"work-1",self.output,kind="work-initial")
        self.assertEqual(result["status"],"COMPLETE")
        self.assertEqual(json.loads((self.output/"items.json").read_text()),records)
        self.assertNotIn("record",json.loads((self.output/"items.json").read_text())[0])
        self.assertEqual(result["rowKinds"],{"COVERAGE":1,"ENTRYPOINT":1,"SOURCE":1})
        self.output=self.base/"proposal-flat-rejected"
        self.assertEqual(self.collect({"FIRST":{"page":page([coverage])}})["reason"],"INVALID_ITEM")

    def test_work_initial_rejects_malformed_envelopes_without_changing_other_guards(self):
        for record in (None,[],{}, {"kind":"COVERAGE"},{"kind":"COVERAGE","id":None},
                       {"kind":2,"id":"sample"},{"kind":"","id":"sample"},
                       {"kind":"COVERAGE","id":""}):
            with self.subTest(record=record),tempfile.TemporaryDirectory() as directory:
                self.output=Path(directory)/"collection";self.driver({"FIRST":{"page":work_page([record])}})
                result=helper.collect(self.launcher,self.root,"work-1",self.output,kind="work-initial")
                self.assertEqual(result["reason"],"INVALID_ITEM")
                self.assertTrue((self.output/"page-0001.stdout").is_file())

    def test_exact_work_references_repeat_selection_with_cursor_and_keep_guards(self):
        self.driver({"FIRST":{"page":work_page([row("DEPENDENCY","d1")],"next",2)},
                     "next":{"page":work_page([row("SOURCE","s1")],total=2,receiptDigest="sha256:second")}})
        result=helper.collect(self.launcher,self.root,"work-1",self.output,kind="work-evidence",references=["d1"])
        self.assertEqual(result["status"],"COMPLETE")
        self.assertEqual(result["readScope"],"EXACT_REFERENCES")
        self.assertEqual(json.loads((self.output/"page-0001.selection.json").read_text()),{"references":["d1"]})
        self.assertEqual(json.loads((self.output/"page-0002.selection.json").read_text()),{"references":["d1"],"cursor":"next"})
        for references in (None,[],[""],["d1","d1"],["d1"]*9):
            with self.subTest(references=references),self.assertRaises(helper.Incomplete):
                helper.collect(self.launcher,self.root,"work-1",self.base/"never-created",kind="work-evidence",references=references)


if __name__ == "__main__":
    unittest.main()
