#!/usr/bin/env ruby
# frozen_string_literal: true

# Planning-side bootstrap acceptance controller for A00/R01/R02/R03/GK.
#
# Runtime usage (fail closed):
#   ruby 2026-08-08-codeclew-bootstrap-controller-v0.rb \
#     --verify controller-case.json
#
# Planning verification:
#   ruby 2026-08-08-codeclew-bootstrap-controller-v0.rb --self-test
#
# TEST_ONLY mode is accepted exclusively inside --self-test. A normal --verify
# invocation requires CODEX_READ_ONLY_APPROVAL_V1 with two independent
# read-thread observations and one scope-conformance receipt. This is
# operational provenance for a local single-user workflow, not a cryptographic
# third-party identity claim.

require "base64"
require "digest"
require "json"
require "open3"
require "openssl"
require "optparse"
require "tempfile"
require "time"

module CodeclewBootstrapV0
  SHA256_PATTERN = /\A[a-f0-9]{64}\z/.freeze
  PLACEHOLDER_PATTERN = /\$\{([A-Z][A-Z0-9_]*)\}/.freeze
  GENERIC_OUTCOMES = %w[FAILURE REFUSED BLOCKED NO_PROGRESS INFRA_ERROR].freeze
  TOKEN_FIELDS = %w[inputTokens cachedInputTokens outputTokens noncachedTokens].freeze
  TOKEN_VERDICTS = %w[WIN LOSS NO_WIN INCONCLUSIVE UNAVAILABLE NOT_EVALUATED].freeze
  PRESENCE_VALUES = %w[REQUIRED OPTIONAL FORBIDDEN].freeze

  class Reject < StandardError
    attr_reader :code, :details

    def initialize(code, message, details = nil)
      super(message)
      @code = code
      @details = details
    end
  end

  module Util
    module_function

    def assert(condition, code, message, details = nil)
      raise Reject.new(code, message, details) unless condition
    end

    def deep_copy(value)
      JSON.parse(JSON.generate(value))
    end

    def exact_set?(left, right)
      left.is_a?(Array) && right.is_a?(Array) && left.length == left.uniq.length &&
        right.length == right.uniq.length && left.sort == right.sort
    end

    def parse_time(value, field)
      Time.iso8601(value)
    rescue ArgumentError, TypeError
      raise Reject.new("INVALID_TIMESTAMP", "#{field} is not an ISO-8601 timestamp", value)
    end

    def digest_ref_key(ref)
      assert(ref.is_a?(Hash), "INVALID_DIGEST_REF", "digest ref must be an object", ref)
      name = ref["ref"]
      digest = ref["sha256"]
      assert(name.is_a?(String) && !name.empty?, "INVALID_DIGEST_REF", "digest ref name is missing", ref)
      assert(digest.is_a?(String) && SHA256_PATTERN.match?(digest), "INVALID_DIGEST_REF", "digest ref sha256 is invalid", ref)
      [name, digest]
    end

    def without_key(hash, key)
      copy = deep_copy(hash)
      copy.delete(key)
      copy
    end
  end

  module CanonicalJSON
    module_function

    # Bootstrap JSON intentionally contains only integer numeric telemetry.
    # Rejecting Float avoids silently claiming RFC 8785 compliance with a
    # platform-dependent binary64 serializer.
    def dump(value)
      case value
      when Hash
        keys = value.keys
        Util.assert(keys.all? { |key| key.is_a?(String) }, "NON_STRING_JSON_KEY", "canonical JSON requires string object keys")
        sorted = keys.sort_by { |key| key.encode("UTF-16BE").bytes }
        "{" + sorted.map { |key| JSON.generate(key) + ":" + dump(value.fetch(key)) }.join(",") + "}"
      when Array
        "[" + value.map { |item| dump(item) }.join(",") + "]"
      when String
        JSON.generate(value)
      when Integer
        value.to_s
      when Float
        raise Reject.new("FLOAT_NOT_SUPPORTED", "bootstrap canonical JSON rejects Float values", value)
      when TrueClass
        "true"
      when FalseClass
        "false"
      when NilClass
        "null"
      else
        raise Reject.new("UNSUPPORTED_JSON_VALUE", "unsupported canonical JSON value", value.class.name)
      end
    end

    def sha256(value)
      Digest::SHA256.hexdigest(dump(value))
    end
  end

  class Expansion
    attr_reader :values

    def initialize(values)
      @values = values
    end
  end

  class Materializer
    MAX_DEPTH = 32

    def initialize(bindings)
      Util.assert(bindings.is_a?(Hash), "INVALID_BINDINGS", "manifest bindings must be an object")
      @bindings = bindings
      @resolving = []
    end

    def materialize(value)
      result = expand(value, 0)
      Util.assert(!result.is_a?(Expansion), "INVALID_TOP_LEVEL_EXPANSION", "top-level manifest cannot expand to siblings")
      ensure_closed(result)
      result
    end

    private

    def expand(value, depth)
      Util.assert(depth <= MAX_DEPTH, "PLACEHOLDER_DEPTH_EXCEEDED", "placeholder expansion exceeded #{MAX_DEPTH} levels")
      case value
      when Hash
        value.each_with_object({}) do |(key, child), output|
          expanded = expand(child, depth + 1)
          Util.assert(!expanded.is_a?(Expansion), "INVALID_OBJECT_EXPANSION", "placeholder expansion cannot create sibling object fields", key)
          output[key] = expanded
        end
      when Array
        value.each_with_object([]) do |child, output|
          expanded = expand(child, depth + 1)
          if expanded.is_a?(Expansion)
            output.concat(expanded.values)
          else
            output << expanded
          end
        end
      when String
        expand_string(value, depth + 1)
      else
        value
      end
    end

    def expand_string(value, depth)
      names = value.scan(PLACEHOLDER_PATTERN).flatten.uniq
      return value if names.empty?

      exact = value.match(/\A\$\{([A-Z][A-Z0-9_]*)\}\z/)
      if exact
        bound = resolve_binding(exact[1], depth)
        return Expansion.new(bound.map { |item| expand(item, depth + 1) }) if bound.is_a?(Array)
        return expand(bound, depth + 1)
      end

      choices = names.map do |name|
        bound = resolve_binding(name, depth)
        values = bound.is_a?(Array) ? bound : [bound]
        Util.assert(values.all? { |item| scalar?(item) }, "NON_SCALAR_EMBEDDED_BINDING", "embedded placeholder #{name} must be scalar")
        [name, values]
      end
      products = [[]]
      choices.each do |name, values|
        products = products.flat_map { |prefix| values.map { |item| prefix + [[name, item]] } }
      end
      expanded = products.map do |assignments|
        rendered = value.dup
        assignments.each { |name, item| rendered = rendered.gsub("${#{name}}", item.to_s) }
        expand_string(rendered, depth + 1)
      end
      expanded.length == 1 ? expanded.first : Expansion.new(expanded)
    end

    def resolve_binding(name, depth)
      Util.assert(@bindings.key?(name), "UNKNOWN_PLACEHOLDER", "manifest placeholder #{name} has no binding")
      Util.assert(!@resolving.include?(name), "PLACEHOLDER_CYCLE", "cyclic placeholder binding", @resolving + [name])
      @resolving << name
      result = expand(@bindings.fetch(name), depth + 1)
      @resolving.pop
      result
    end

    def scalar?(value)
      value.is_a?(String) || value.is_a?(Integer) || value == true || value == false
    end

    def ensure_closed(value)
      case value
      when Hash
        value.each_value { |child| ensure_closed(child) }
      when Array
        value.each { |child| ensure_closed(child) }
      when String
        Util.assert(value.scan(PLACEHOLDER_PATTERN).empty?, "UNRESOLVED_PLACEHOLDER", "materialized manifest still contains a placeholder", value)
      end
    end
  end

  class ArtifactPolicy
    attr_reader :records

    def initialize(manifest, outcome, branch_code)
      field = GENERIC_OUTCOMES.include?(outcome) ? "genericOutcomeArtifacts" : "outputArtifacts"
      entries = manifest[field]
      Util.assert(entries.is_a?(Array) && !entries.empty?, "INVALID_ARTIFACT_POLICY", "manifest #{field} must be a non-empty array")
      @records = entries.map { |entry| evaluate(entry, outcome, branch_code) }
      paths = @records.map { |record| record.fetch("path") }
      Util.assert(paths.length == paths.uniq.length, "DUPLICATE_OUTPUT_PATH", "materialized output artifact paths must be unique", paths)
    end

    def packet_records
      @records.reject { |record| record["selfReferential"] }
    end

    def required_paths
      packet_records.select { |record| record["presence"] == "REQUIRED" }.map { |record| record["path"] }.sort
    end

    def allowed_paths
      packet_records.reject { |record| record["presence"] == "FORBIDDEN" }.map { |record| record["path"] }.sort
    end

    def forbidden_paths
      packet_records.select { |record| record["presence"] == "FORBIDDEN" }.map { |record| record["path"] }.sort
    end

    def output_set_digest
      CanonicalJSON.sha256(allowed_paths)
    end

    private

    def evaluate(entry, outcome, branch_code)
      if entry.is_a?(String)
        return {
          "path" => entry,
          "selfReferential" => self_referential_path?(entry),
          "presence" => "REQUIRED"
        }
      end

      Util.assert(entry.is_a?(Hash), "INVALID_ARTIFACT_POLICY", "output artifact must be a string or policy object", entry)
      path = entry["path"]
      Util.assert(path.is_a?(String) && !path.empty?, "INVALID_ARTIFACT_POLICY", "output artifact path is missing", entry)
      policy = entry["policy"] || {}
      Util.assert(policy.is_a?(Hash), "INVALID_ARTIFACT_POLICY", "artifact policy must be an object", entry)
      default_presence = policy["default"] || entry["presence"] || "REQUIRED"
      rules = policy["rules"] || []
      Util.assert(rules.is_a?(Array), "INVALID_ARTIFACT_POLICY", "artifact policy rules must be an array", entry)
      matching = rules.select { |rule| rule_matches?(rule, outcome, branch_code) }
      Util.assert(matching.length <= 1, "AMBIGUOUS_ARTIFACT_POLICY", "more than one artifact policy rule matched", { "path" => path, "rules" => matching })
      presence = matching.empty? ? default_presence : matching.first["presence"]
      Util.assert(PRESENCE_VALUES.include?(presence), "INVALID_ARTIFACT_POLICY", "artifact presence must be REQUIRED, OPTIONAL, or FORBIDDEN", entry)
      {
        "path" => path,
        "selfReferential" => entry.key?("selfReferential") ? entry["selfReferential"] : self_referential_path?(path),
        "presence" => presence
      }
    end

    def rule_matches?(rule, outcome, branch_code)
      Util.assert(rule.is_a?(Hash) && rule["presence"], "INVALID_ARTIFACT_POLICY", "artifact rule must contain presence", rule)
      condition = rule["when"] || {}
      outcomes = condition["outcomes"]
      branches = condition["branchCodes"]
      (outcomes.nil? || outcomes.include?(outcome)) && (branches.nil? || branches.include?(branch_code))
    end

    def self_referential_path?(path)
      path.end_with?("/packet.json") || path.end_with?("/summary.md")
    end
  end

  class DotEdgeRegistry
    AUTHORIZATION_FIELDS = %w[
      id source target acceptedOutcomes acceptedBranchCodes branchMatch retryState gatePermitted
    ].freeze

    def self.for_source(dot, source)
      Util.assert(dot.is_a?(String) && dot.valid_encoding?, "INVALID_APPROVED_DOT", "approved DOT must be valid UTF-8")
      default_match = dot.match(/^\s*edge\s*\[(.*?)\]\s*;/m)
      Util.assert(!default_match.nil?, "INVALID_APPROVED_DOT", "approved DOT lacks a default edge contract")
      defaults = attributes(default_match[1])
      edges = []
      pattern = /^\s*([A-Z][A-Z0-9]*)\s*->\s*([A-Z][A-Z0-9]*)\s*(?:\[(.*?)\])?\s*;/m
      dot.scan(pattern) do |from, target, raw_attributes|
        next unless from == source
        merged = defaults.merge(attributes(raw_attributes.to_s))
        edges << {
          "id" => "#{from}->#{target}",
          "source" => from,
          "target" => target,
          "acceptedOutcomes" => csv(merged.fetch("acceptedOutcomes")),
          "acceptedBranchCodes" => csv(merged.fetch("acceptedBranchCodes")),
          "branchMatch" => merged.fetch("branchMatch"),
          "retryState" => merged.fetch("retryState"),
          "gatePermitted" => true
        }
      end
      ids = edges.map { |edge| edge["id"] }
      Util.assert(!edges.empty? && ids.length == ids.uniq.length, "INVALID_APPROVED_DOT",
                  "approved bootstrap DOT edges must have unique source/target IDs", { "source" => source, "ids" => ids })
      edges.sort_by { |edge| edge["id"] }
    end

    def self.attributes(text)
      text.scan(/([A-Za-z][A-Za-z0-9_]*)\s*=\s*("(?:\\.|[^"])*"|[^,\s]+)/m).each_with_object({}) do |(key, raw), result|
        result[key] = raw.start_with?("\"") ? JSON.parse(raw) : raw
      end
    rescue JSON::ParserError => error
      raise Reject.new("INVALID_APPROVED_DOT", "approved DOT contains an invalid quoted attribute", error.message)
    end

    def self.csv(value)
      value.to_s.split(",").reject(&:empty?)
    end

    private_class_method :attributes, :csv
  end

  module RetryAuthorization
    module_function

    def build(entry, packet, receipt, manifest)
      payload = {
        "decision" => "AUTHORIZED_BY_CONTROLLER_POLICY",
        "nodeId" => packet["nodeId"],
        "fromAttempt" => entry["attempt"],
        "toAttempt" => entry["attempt"] + 1,
        "priorOutcome" => packet["outcome"],
        "priorBranchCode" => packet["branchCode"],
        "priorPacketDigest" => CanonicalJSON.sha256(packet),
        "priorReceiptDigest" => CanonicalJSON.sha256(Util.without_key(receipt, "receiptDigest")),
        "priorSelfArtifactsDigest" => CanonicalJSON.sha256(
          "packetSelfRef" => entry["packetSelfRef"],
          "summarySelfRef" => entry["summarySelfRef"]
        ),
        "retryableGenericBranchCodesDigest" => CanonicalJSON.sha256(manifest["retryableGenericBranchCodes"])
      }
      payload.merge("authorizationDigest" => CanonicalJSON.sha256(payload))
    end
  end

  class ObjectStore
    def initialize(entries)
      Util.assert(entries.is_a?(Array), "INVALID_OBJECT_STORE", "objectStore must be an array")
      @by_ref = {}
      entries.each do |entry|
        Util.assert(entry.is_a?(Hash), "INVALID_OBJECT_STORE", "objectStore entry must be an object", entry)
        ref = entry["ref"]
        Util.assert(ref.is_a?(String) && !ref.empty?, "INVALID_OBJECT_STORE", "objectStore ref is missing", entry)
        Util.assert(!@by_ref.key?(ref), "DUPLICATE_OBJECT_REF", "objectStore ref must be unique", ref)
        computed = digest(entry)
        claimed = entry["sha256"]
        Util.assert(claimed == computed, "OBJECT_DIGEST_MISMATCH", "objectStore content digest mismatch", { "ref" => ref, "claimed" => claimed, "computed" => computed })
        @by_ref[ref] = entry
      end
    end

    def resolve(ref_object)
      ref, digest = Util.digest_ref_key(ref_object)
      entry = @by_ref[ref]
      Util.assert(!entry.nil?, "DANGLING_DIGEST_REF", "objectStore does not contain #{ref}")
      Util.assert(entry["sha256"] == digest, "OBJECT_DIGEST_MISMATCH", "digest ref does not match objectStore", ref_object)
      entry
    end

    def json(ref_object)
      entry = resolve(ref_object)
      Util.assert(%w[RFC8785_JSON RECEIPT_WITHOUT_RECEIPT_DIGEST].include?(entry["encoding"]), "OBJECT_NOT_JSON", "object is not stored as JSON", ref_object)
      Util.deep_copy(entry["content"])
    end

    def receipt(ref_object)
      entry = resolve(ref_object)
      Util.assert(entry["encoding"] == "RECEIPT_WITHOUT_RECEIPT_DIGEST", "OBJECT_NOT_RECEIPT",
                  "parent receipt must use the frozen receipt digest scope", ref_object)
      Util.deep_copy(entry["content"])
    end

    def utf8(ref_object)
      entry = resolve(ref_object)
      Util.assert(entry["encoding"] == "UTF8", "OBJECT_NOT_UTF8", "object must use UTF8 encoding", ref_object)
      entry["content"].dup
    end

    def ref_for(name)
      entry = @by_ref[name]
      Util.assert(!entry.nil?, "DANGLING_DIGEST_REF", "objectStore does not contain #{name}")
      { "ref" => name, "sha256" => entry["sha256"] }
    end

    def json_document(ref_object)
      entry = resolve(ref_object)
      case entry["encoding"]
      when "RFC8785_JSON", "RECEIPT_WITHOUT_RECEIPT_DIGEST"
        Util.deep_copy(entry["content"])
      when "UTF8"
        JSON.parse(entry["content"])
      else
        raise Reject.new("OBJECT_NOT_JSON", "object is not stored as a JSON document", ref_object)
      end
    rescue JSON::ParserError
      raise Reject.new("OBJECT_NOT_JSON", "UTF8 object is not valid JSON", ref_object)
    end

    private

    def digest(entry)
      case entry["encoding"]
      when "RFC8785_JSON"
        CanonicalJSON.sha256(entry["content"])
      when "RECEIPT_WITHOUT_RECEIPT_DIGEST"
        content = entry["content"]
        Util.assert(content.is_a?(Hash), "INVALID_OBJECT_STORE", "receipt object content must be an object", entry["ref"])
        CanonicalJSON.sha256(Util.without_key(content, "receiptDigest"))
      when "UTF8"
        content = entry["content"]
        Util.assert(content.is_a?(String) && content.encoding == Encoding::UTF_8 && content.valid_encoding?,
                    "INVALID_UTF8_OBJECT", "UTF8 object content must be a valid UTF-8 string", entry["ref"])
        Digest::SHA256.hexdigest(content)
      when "BASE64"
        Digest::SHA256.hexdigest(Base64.strict_decode64(entry["content"].to_s))
      else
        raise Reject.new("INVALID_OBJECT_ENCODING", "unknown objectStore encoding", entry["encoding"])
      end
    rescue ArgumentError
      raise Reject.new("INVALID_OBJECT_ENCODING", "invalid base64 objectStore content", entry["ref"])
    end
  end

  class Controller
    APPROVED_SOURCE_ROLES = %w[S0 S1 S2 S3 S4 S5].freeze
    attr_reader :last_materialized_manifest

    def initialize(self_test: false)
      @self_test = self_test
    end

    def verify(case_data)
      Util.assert(case_data.is_a?(Hash), "INVALID_CONTROLLER_CASE", "controller case must be an object")
      @mode = case_data["mode"]
      Util.assert(%w[CODEX_READ_ONLY_APPROVAL_V1 TEST_ONLY].include?(@mode), "INVALID_CONTROLLER_MODE", "controller mode is invalid")
      Util.assert(@self_test || @mode == "CODEX_READ_ONLY_APPROVAL_V1", "TEST_ONLY_FORBIDDEN", "TEST_ONLY input is accepted only by --self-test")

      @store = ObjectStore.new(case_data["objectStore"] || [])
      @approval_bundle = case_data["approvalBundle"]
      @runtime = case_data["runtimeState"]
      @packet = case_data["packet"]
      @receipt = case_data["receipt"]
      @self_artifacts = case_data["selfArtifacts"]
      @manifest_bundle = case_data["manifestBundle"]
      @schema_paths = case_data["schemaPaths"]
      Util.assert(@approval_bundle.is_a?(Hash), "INVALID_APPROVAL_BUNDLE", "approvalBundle is missing")
      Util.assert(@runtime.is_a?(Hash), "INVALID_RUNTIME_STATE", "runtimeState is missing")
      Util.assert(@packet.is_a?(Hash), "INVALID_PACKET", "packet is missing")
      Util.assert(@receipt.is_a?(Hash), "INVALID_RECEIPT", "receipt is missing")
      Util.assert(@manifest_bundle.is_a?(Hash), "INVALID_MANIFEST_BUNDLE", "manifestBundle is missing")

      verify_runtime_attestation!(case_data["runtimeAttestation"])
      verify_approval_bundle!
      validate_schema_inputs!(@schema_paths) unless @self_test
      @manifest = materialize_manifest!(case_data["bindings"] || {})
      verify_packet_and_receipt_identity!
      verify_predecessors_and_sources!
      verify_artifacts_and_evidence!
      cost = verify_cost_accounting!
      verify_metric_eligibility!(cost)
      verify_gk_wave! if @packet["nodeId"] == "GK"
      effective = effective_projection(cost)
      edges = derive_effective_edges(effective)

      {
        "controllerVerdict" => "CONTROL_ACCEPT",
        "effectiveOutcome" => effective.fetch("outcome"),
        "effectiveBranchCode" => effective.fetch("branchCode"),
        "effectiveEligibleNextEdges" => edges,
        "producerEdgeHintMatches" => exact_string_set?(@packet["proposedNextEdges"] || [], edges),
        "budgetStatus" => cost.fetch("budgetStatus"),
        "materializedManifestDigest" => CanonicalJSON.sha256(@manifest),
        "artifactSetDigest" => @packet["artifactSetDigest"]
      }
    end

    private

    def verify_runtime_attestation!(attestation)
      Util.assert(attestation.is_a?(Hash), "MISSING_RUNTIME_ATTESTATION", "runtimeAttestation is required")
      digest = CanonicalJSON.sha256(@runtime)
      Util.assert(attestation["stateDigest"] == digest, "RUNTIME_STATE_DIGEST_MISMATCH", "runtimeState digest does not match attestation")
      Util.assert(attestation["provenance"] == "INDEPENDENT_RUNTIME_OBSERVATION_V1",
                  "INVALID_RUNTIME_PROVENANCE", "runtime state requires independent read-only observations")
      observations = attestation["observations"]
      Util.assert(observations.is_a?(Array) && observations.length == 2,
                  "RUNTIME_OBSERVATION_COUNT_MISMATCH", "exactly two runtime observations are required")
      observations.each do |observation|
        Util.assert(observation.is_a?(Hash) &&
                    observation["source"] == "READ_ONLY_EVIDENCE_AND_GIT_STATE" &&
                    observation["decision"] == "OBSERVED_EXACT_RUNTIME_STATE" &&
                    observation["stateDigest"] == digest,
                    "INVALID_RUNTIME_OBSERVATION", "runtime observation is not bound to the exact state")
        receipt_digest = CanonicalJSON.sha256(Util.without_key(observation, "receiptDigest"))
        Util.assert(observation["receiptDigest"] == receipt_digest,
                    "RUNTIME_OBSERVATION_RECEIPT_DIGEST_MISMATCH", "runtime observation receipt is not reproducible")
        Util.parse_time(observation["observedAt"], "runtimeObservation.observedAt")
      end
      @runtime_observer_sessions = observations.map { |observation| observation["observerSessionId"] }
      Util.assert(@runtime_observer_sessions.all? { |id| id.is_a?(String) && !id.empty? } &&
                  @runtime_observer_sessions.uniq.length == 2,
                  "RUNTIME_OBSERVER_INDEPENDENCE_VIOLATION", "runtime observer sessions must be present and pairwise distinct")
    end

    def verify_approval_bundle!
      subject = @approval_bundle["approvalSubject"]
      decision = @approval_bundle["humanDecision"]
      Util.assert(subject.is_a?(Hash) && decision.is_a?(Hash), "INVALID_APPROVAL_BUNDLE", "approval subject or human decision is missing")
      subject_digest = CanonicalJSON.sha256(subject)
      Util.assert(@approval_bundle["approvalSubjectDigest"] == subject_digest, "APPROVAL_SUBJECT_DIGEST_MISMATCH", "approvalSubjectDigest is not reproducible")
      Util.assert(decision["decision"] == "HUMAN_APPROVED", "HUMAN_APPROVAL_MISSING", "human decision is not HUMAN_APPROVED")
      Util.assert(decision["subjectDigest"] == subject_digest, "HUMAN_DECISION_SUBJECT_MISMATCH", "human decision signs a different subject")
      actor = decision["actor"] || {}
      Util.assert(actor["type"] == "HUMAN" && actor["subjectId"].is_a?(String) && !actor["subjectId"].empty?, "INVALID_HUMAN_ACTOR", "human decision actor is invalid")
      verify_session_approval!(decision["sessionEvidence"])

      decision_ref = @approval_bundle["humanDecisionRef"]
      Util.assert(decision_ref.is_a?(Hash) && decision_ref["role"] == "HUMAN_DECISION", "INVALID_HUMAN_DECISION_REF", "humanDecisionRef is invalid")
      Util.assert(decision_ref["sha256"] == CanonicalJSON.sha256(decision), "HUMAN_DECISION_DIGEST_MISMATCH", "embedded human decision digest does not match humanDecisionRef")
      stored_decision = @store.json(decision_ref)
      Util.assert(CanonicalJSON.dump(stored_decision) == CanonicalJSON.dump(decision), "HUMAN_DECISION_OBJECT_MISMATCH", "stored human decision differs from embedded decision")

      refs = []
      refs << subject["plan"]
      refs << subject["authoritativeDot"]
      refs.concat(subject["bootstrapArtifacts"] || [])
      refs << subject["planningVerificationReport"]
      refs.concat(subject["sources"] || [])
      Util.assert(refs.all? { |ref| ref.is_a?(Hash) }, "INVALID_APPROVAL_SUBJECT", "approval subject contains a malformed artifact ref")
      roles = refs.map { |ref| ref["role"] }
      names = refs.map { |ref| ref["ref"] }
      Util.assert(roles.length == roles.uniq.length, "DUPLICATE_APPROVAL_ROLE", "approval subject roles must be unique", roles)
      Util.assert(names.length == names.uniq.length, "DUPLICATE_APPROVAL_REF", "approval subject refs must be unique", names)
      refs.each { |ref| @store.resolve(ref) }

      manifest_ref = (subject["bootstrapArtifacts"] || []).find { |ref| ref["role"] == "BOOTSTRAP_MANIFESTS" }
      Util.assert(!manifest_ref.nil?, "APPROVED_MANIFEST_MISSING", "approval subject does not freeze BOOTSTRAP_MANIFESTS")
      approved_manifest_bundle = @store.json_document(manifest_ref)
      Util.assert(CanonicalJSON.dump(approved_manifest_bundle) == CanonicalJSON.dump(@manifest_bundle),
                  "APPROVED_MANIFEST_BUNDLE_MISMATCH", "runtime manifest bundle differs from the signed BOOTSTRAP_MANIFESTS artifact")
      @approved_dot = @store.utf8(subject["authoritativeDot"])
      controller_ref = (subject["bootstrapArtifacts"] || []).find { |ref| ref["role"] == "BOOTSTRAP_CONTROLLER" }
      Util.assert(!controller_ref.nil?, "APPROVED_CONTROLLER_MISSING", "approval subject does not freeze BOOTSTRAP_CONTROLLER")
      executing_digest = Digest::SHA256.file(File.expand_path(__FILE__)).hexdigest
      Util.assert(controller_ref["sha256"] == executing_digest, "EXECUTING_CONTROLLER_DIGEST_MISMATCH",
                  "executing bootstrap controller differs from the human-approved controller artifact")

      plan_status = subject["planStatus"]
      Util.assert(plan_status == "PROPOSED_AWAITING_HUMAN_APPROVAL" && @approval_bundle["planStatus"] == plan_status,
                  "APPROVAL_PLAN_STATUS_MISMATCH", "approval applies to the wrong plan status")
    end

    def verify_session_approval!(evidence)
      Util.assert(evidence.is_a?(Hash) && evidence["provenance"] == "CURRENT_CODEX_SESSION",
                  "SESSION_APPROVAL_MISSING", "human approval is not recorded from the current Codex session")
      messages = evidence["messages"]
      Util.assert(messages.is_a?(Array) && !messages.empty?, "SESSION_APPROVAL_MISSING", "no user approval message is recorded")
      thread_id = evidence["threadId"]
      Util.assert(thread_id.is_a?(String) && !thread_id.empty?, "SESSION_APPROVAL_MISSING", "approval thread ID is missing")
      messages.each do |message|
        Util.assert(message["threadId"] == thread_id && message["authorRole"] == "USER",
                    "SESSION_APPROVAL_IDENTITY_MISMATCH", "approval evidence is not a USER message in the recorded thread")
        digest = Digest::SHA256.hexdigest(message["messageText"].to_s)
        Util.assert(message["messageTextDigest"] == digest,
                    "SESSION_APPROVAL_DIGEST_MISMATCH", "approval message digest is not reproducible")
      end
      Util.parse_time(evidence["checkedAt"], "sessionEvidence.checkedAt")
    end

    def validate_schema_inputs!(schema_paths)
      required = %w[approvalBundle nodeContract packet receipt]
      Util.assert(schema_paths.is_a?(Hash) && required.all? { |key| schema_paths[key].is_a?(String) },
                  "SCHEMA_PATHS_REQUIRED", "runtime verification requires approvalBundle/nodeContract/packet/receipt schema paths")
      validate_with_ajv!(schema_paths["approvalBundle"], @approval_bundle, "approvalBundle")
      validate_with_ajv!(schema_paths["nodeContract"], @manifest_bundle, "manifestBundle")
      validate_with_ajv!(schema_paths["packet"], @packet, "packet")
      validate_with_ajv!(schema_paths["receipt"], @receipt, "receipt")

      role_by_key = {
        "approvalBundle" => "APPROVAL_BUNDLE_SCHEMA",
        "nodeContract" => "NODE_CONTRACT_SCHEMA",
        "packet" => "EVIDENCE_PACKET_SCHEMA",
        "receipt" => "VERIFICATION_RECEIPT_SCHEMA"
      }
      frozen = (@approval_bundle.dig("approvalSubject", "bootstrapArtifacts") || []).each_with_object({}) do |ref, index|
        index[ref["role"]] = ref
      end
      role_by_key.each do |key, role|
        ref = frozen[role]
        Util.assert(!ref.nil?, "SCHEMA_NOT_APPROVED", "approval subject does not contain #{role}")
        actual = Digest::SHA256.file(schema_paths.fetch(key)).hexdigest
        Util.assert(actual == ref["sha256"], "SCHEMA_DIGEST_MISMATCH", "#{role} file digest changed after approval")
      end
    end

    def validate_with_ajv!(schema_path, value, label)
      Util.assert(File.file?(schema_path), "SCHEMA_FILE_MISSING", "#{label} schema file is missing", schema_path)
      Tempfile.create(["codeclew-#{label}", ".json"]) do |file|
        file.write(JSON.generate(value))
        file.flush
        command = ["npx", "--yes", "ajv-cli@5", "validate", "--spec=draft2020", "-s", schema_path, "-d", file.path]
        stdout, stderr, status = Open3.capture3(*command)
        Util.assert(status.success?, "SCHEMA_VALIDATION_FAILED", "#{label} is not schema-valid", { "stdout" => stdout, "stderr" => stderr })
      end
    end

    def materialize_manifest!(bindings)
      manifests = @manifest_bundle["manifests"]
      Util.assert(manifests.is_a?(Array), "INVALID_MANIFEST_BUNDLE", "manifestBundle.manifests is missing")
      matches = manifests.select { |manifest| manifest["id"] == @packet["nodeId"] }
      Util.assert(matches.length == 1, "MANIFEST_ID_CARDINALITY", "packet node must have exactly one manifest", @packet["nodeId"])

      subject = @approval_bundle["approvalSubject"]
      plan_digest = subject.dig("plan", "sha256")
      effective_bindings = Util.deep_copy(bindings)
      effective_bindings["PLAN_DIGEST"] ||= plan_digest
      effective_bindings["ATTEMPT"] ||= @packet["attempt"].to_s
      Util.assert(effective_bindings["PLAN_DIGEST"] == plan_digest, "PLAN_BINDING_MISMATCH", "PLAN_DIGEST binding is not the approved plan")
      Util.assert(effective_bindings["ATTEMPT"].to_s == @packet["attempt"].to_s, "ATTEMPT_BINDING_MISMATCH", "ATTEMPT binding does not match packet attempt")
      @effective_bindings = effective_bindings
      materialized = Materializer.new(effective_bindings).materialize(matches.first)
      retryable_codes = materialized["retryableGenericBranchCodes"]
      branch_codes = materialized["branchCodes"] || []
      Util.assert(retryable_codes.is_a?(Array) && retryable_codes.length == retryable_codes.uniq.length &&
                  retryable_codes.all? { |code| branch_codes.include?(code) } &&
                  (retryable_codes & %w[NONE BUDGET_EXCEEDED]).empty?,
                  "INVALID_RETRY_BRANCH_POLICY", "retryableGenericBranchCodes must be an exact safe subset of branchCodes")
      @last_materialized_manifest = materialized
      digest = CanonicalJSON.sha256(materialized)
      Util.assert(@packet["runManifestDigest"] == digest, "RUN_MANIFEST_DIGEST_MISMATCH", "packet runManifestDigest is not the materialized manifest digest", { "expected" => digest, "actual" => @packet["runManifestDigest"] })
      materialized
    end

    def verify_packet_and_receipt_identity!
      node = @packet["nodeId"]
      Util.assert(node == @manifest["id"] && @receipt["nodeId"] == node, "NODE_ID_MISMATCH", "manifest, packet, and receipt node IDs differ")
      Util.assert(@receipt["attempt"] == @packet["attempt"], "ATTEMPT_MISMATCH", "receipt attempt differs from packet attempt")
      Util.assert(@packet["attempt"].is_a?(Integer) && @packet["attempt"].between?(1, @manifest.dig("retryPolicy", "maxProducerAttempts").to_i),
                  "ATTEMPT_OUT_OF_RANGE", "packet attempt exceeds manifest retry policy")
      approval_digest = CanonicalJSON.sha256(@approval_bundle)
      Util.assert(@packet["approvalBundleDigest"] == approval_digest && @receipt["approvalBundleDigest"] == approval_digest,
                  "APPROVAL_BUNDLE_DIGEST_MISMATCH", "packet or receipt approval bundle digest mismatch")
      Util.assert(@receipt["runManifestDigest"] == @packet["runManifestDigest"], "RUN_MANIFEST_DIGEST_MISMATCH", "receipt runManifestDigest differs from packet")

      packet_digest = CanonicalJSON.sha256(@packet)
      Util.assert(@receipt["packetDigest"] == packet_digest, "PACKET_DIGEST_MISMATCH", "receipt packetDigest is not reproducible")
      receipt_digest = CanonicalJSON.sha256(Util.without_key(@receipt, "receiptDigest"))
      Util.assert(@receipt["receiptDigest"] == receipt_digest, "RECEIPT_DIGEST_MISMATCH", "receiptDigest is not reproducible")
      Util.assert(@receipt["digestScope"] == "RFC8785_CANONICAL_JSON_WITHOUT_RECEIPT_DIGEST", "RECEIPT_DIGEST_SCOPE_MISMATCH", "receipt digestScope is invalid")
      Util.assert(@receipt["packetOutcome"] == @packet["outcome"] && @receipt["packetBranchCode"] == @packet["branchCode"],
                  "PACKET_RECEIPT_OUTCOME_MISMATCH", "receipt outcome or branch differs from packet")
      Util.assert(@manifest["allowedOutcomes"].include?(@packet["outcome"]), "UNREGISTERED_OUTCOME", "packet outcome is not registered in manifest")
      Util.assert(@manifest["branchCodes"].include?(@packet["branchCode"]), "UNREGISTERED_BRANCH_CODE", "packet branch code is not registered in manifest")
      Util.assert(Util.exact_set?(@packet["hypothesisIds"], @manifest["hypothesisIds"]), "HYPOTHESIS_SET_MISMATCH", "packet hypothesis set differs from manifest")

      producer_session = @packet.dig("producer", "sessionId")
      verifier_session = @receipt.dig("verifier", "sessionId")
      Util.assert(@receipt["producerSessionId"] == producer_session, "PRODUCER_SESSION_MISMATCH", "receipt producerSessionId differs from packet producer")
      Util.assert(verifier_session != producer_session, "SESSION_INDEPENDENCE_VIOLATION", "producer and verifier sessions must differ")
      Util.assert(@receipt["independenceAttestation"] == true, "SESSION_INDEPENDENCE_VIOLATION", "receipt lacks independence attestation")
      authority_sessions = (@approval_observer_sessions || []) + (@runtime_observer_sessions || [])
      Util.assert(authority_sessions.uniq.length == authority_sessions.length &&
                  !authority_sessions.include?(producer_session) && !authority_sessions.include?(verifier_session),
                  "AUTHORITY_SESSION_INDEPENDENCE_VIOLATION",
                  "approval/runtime authority observers must be pairwise distinct from producer and node verifier")

      checks = @receipt["checks"] || []
      check_ids = checks.map { |check| check["checkId"] }
      Util.assert(check_ids == @manifest["requiredCheckIds"], "MANDATORY_CHECK_SET_MISMATCH", "receipt checks must exactly match manifest requiredCheckIds in frozen order")
      if %w[ACCEPT ACCEPT_EXPLORATORY_ONLY].include?(@receipt["verdict"])
        Util.assert(checks.all? { |check| check["result"] == "PASS" }, "MANDATORY_CHECK_FAILED", "accepted receipt contains a non-PASS mandatory check")
      end
    end

    def verify_predecessors_and_sources!
      approved_sources = @approval_bundle.dig("approvalSubject", "sources") || []
      current_sources = @runtime["currentSourceDigests"] || []
      packet_sources = @packet["sourceDigests"] || []
      approved_keys = canonical_source_set!(approved_sources, "INVALID_APPROVED_SOURCE_SET", "approvalSubject.sources")
      runtime_keys = canonical_source_set!(current_sources, "INVALID_RUNTIME_SOURCE_SET", "runtimeState.currentSourceDigests")
      packet_keys = canonical_source_set!(packet_sources, "INVALID_PACKET_SOURCE_SET", "packet.sourceDigests")
      Util.assert(runtime_keys == approved_keys, "RUNTIME_APPROVED_SOURCE_SET_MISMATCH",
                  "trusted runtime sources differ from the human-approved S0-S5 source set")
      Util.assert(packet_keys == approved_keys, "SOURCE_APPROVAL_SET_MISMATCH",
                  "packet sources differ from the human-approved S0-S5 source set")
      approved_sources.each { |ref| @store.resolve(ref) }

      expected_parents = @runtime["expectedParentReceiptDigests"] || []
      Util.assert(digest_ref_arrays_equal?(@packet["parentReceiptDigests"], expected_parents), "PARENT_RECEIPT_SET_MISMATCH", "packet parent receipt set differs from trusted runtime state")
      expected_parents.each { |ref| @store.resolve(ref) }
      verify_manifest_hard_predecessors!(expected_parents)
    end

    def verify_manifest_hard_predecessors!(parents)
      requirements = @manifest["hardPredecessors"]
      Util.assert(requirements.is_a?(Array), "INVALID_HARD_PREDECESSORS", "materialized manifest hardPredecessors must be an array")
      case @packet["nodeId"]
      when "R01"
        Util.assert(requirements == ["A00:HUMAN_APPROVED"], "INVALID_HARD_PREDECESSORS", "R01 must have only the A00 human approval predecessor")
        Util.assert(parents.empty?, "PARENT_RECEIPT_SET_MISMATCH", "R01 uses the approval bundle and must not carry a predecessor receipt")
      when "R02", "R03"
        Util.assert(requirements == ["R01:SUCCESS/NONE"], "INVALID_HARD_PREDECESSORS", "#{@packet['nodeId']} must have only R01:SUCCESS/NONE")
        Util.assert(parents.length == 1, "PARENT_RECEIPT_SET_MISMATCH", "#{@packet['nodeId']} requires exactly one R01 receipt")
        receipt = @store.receipt(parents.first)
        digest = CanonicalJSON.sha256(Util.without_key(receipt, "receiptDigest"))
        Util.assert(receipt["receiptDigest"] == digest && parents.first["sha256"] == digest,
                    "PARENT_RECEIPT_DIGEST_MISMATCH", "R01 predecessor receipt digest is not reproducible")
        Util.assert(receipt["nodeId"] == "R01" && receipt["verdict"] == "ACCEPT" &&
                    receipt["packetOutcome"] == "SUCCESS" && receipt["packetBranchCode"] == "NONE",
                    "HARD_PREDECESSOR_NOT_ACCEPTED", "R01 predecessor is not accepted SUCCESS/NONE", receipt)
        Util.assert(receipt["approvalBundleDigest"] == CanonicalJSON.sha256(@approval_bundle),
                    "PARENT_APPROVAL_BUNDLE_MISMATCH", "R01 predecessor belongs to another approval bundle")
      when "GK"
        Util.assert(requirements == ["WAVE_QUIESCENT_EXHAUSTED_SET:R01|R02|R03"], "INVALID_HARD_PREDECESSORS",
                    "GK must consume only the exact quiescent exhausted bootstrap wave")
      else
        raise Reject.new("UNSUPPORTED_BOOTSTRAP_NODE", "bootstrap controller supports only R01/R02/R03/GK", @packet["nodeId"])
      end
    end

    def verify_artifacts_and_evidence!
      policy = ArtifactPolicy.new(@manifest, @packet["outcome"], @packet["branchCode"])
      artifacts = @packet["artifacts"] || []
      paths = artifacts.map { |artifact| artifact["path"] }
      Util.assert(paths.length == paths.uniq.length, "DUPLICATE_PACKET_ARTIFACT", "packet artifact paths must be unique", paths)
      Util.assert(paths.all? { |path| path.is_a?(String) && !path.empty? }, "INVALID_PACKET_ARTIFACT", "packet artifact path is invalid")

      if GENERIC_OUTCOMES.include?(@packet["outcome"])
        missing = policy.required_paths - paths
        Util.assert(missing.empty?, "MISSING_GENERIC_DIAGNOSTIC", "generic outcome packet lacks required genericOutcomeArtifacts", missing)
        success_policy = ArtifactPolicy.new(@manifest.merge("genericOutcomeArtifacts" => @manifest["outputArtifacts"]), "FAILURE", @packet["branchCode"])
        declared = (policy.allowed_paths + success_policy.allowed_paths).uniq
        extra = paths - declared
        Util.assert(extra.empty?, "UNDECLARED_PACKET_ARTIFACT", "generic outcome packet contains undeclared artifacts", extra)
        success_subset = paths & success_policy.allowed_paths
        preexisting = @runtime["preexistingSuccessArtifacts"] || []
        success_subset.each do |path|
          artifact = artifacts.find { |candidate| candidate["path"] == path }
          allowed = preexisting.any? { |ref| ref["ref"] == path && ref["sha256"] == artifact["sha256"] }
          Util.assert(allowed, "UNPROVEN_PREEXISTING_ARTIFACT", "generic packet success-artifact subset was not already immutable", path)
        end
      else
        Util.assert(paths.sort == policy.required_paths, "SUCCESS_ARTIFACT_SET_MISMATCH", "successful packet artifact paths must equal materialized outputArtifacts", { "expected" => policy.required_paths, "actual" => paths.sort })
      end

      actual_digest = CanonicalJSON.sha256(paths.sort)
      Util.assert(@packet["artifactSetDigest"] == actual_digest, "ARTIFACT_SET_DIGEST_MISMATCH", "packet artifactSetDigest is not the exact sorted actual path digest")
      artifact_index = {}
      artifacts.each do |artifact|
        ref = { "ref" => artifact["path"], "sha256" => artifact["sha256"] }
        @store.resolve(ref)
        artifact_index[artifact["path"]] = artifact["sha256"]
      end

      refs = []
      (@packet["claims"] || []).each { |claim| refs.concat(claim["evidenceRefs"] || []) }
      delta = @packet["evidenceDelta"] || {}
      refs.concat(delta["artifactRefs"] || [])
      (@receipt["checks"] || []).each { |check| refs << check["evidenceRef"] }
      authorized = @runtime["authorizedEvidenceRefs"] || []
      refs.each do |ref|
        name, digest = Util.digest_ref_key(ref)
        local = artifact_index[name]
        if local
          Util.assert(local == digest, "EVIDENCE_REF_DIGEST_MISMATCH", "evidence ref conflicts with packet artifact", ref)
        else
          allowed = authorized.any? { |candidate| candidate["ref"] == name && candidate["sha256"] == digest }
          Util.assert(allowed, "UNAUTHORIZED_EVIDENCE_REF", "evidence ref is neither a packet artifact nor authorized predecessor object", ref)
        end
        @store.resolve(ref)
      end
      verify_self_artifacts!(policy)
    end

    def verify_self_artifacts!(policy)
      Util.assert(@self_artifacts.is_a?(Hash) && @self_artifacts.keys.sort == %w[packet summary],
                  "SELF_ARTIFACT_SET_MISMATCH", "controller case must contain exact packet and summary selfArtifacts")
      self_records = policy.records.select { |record| record["selfReferential"] }
      packet_paths = self_records.map { |record| record["path"] }.select { |path| path.end_with?("/packet.json") }
      summary_paths = self_records.map { |record| record["path"] }.select { |path| path.end_with?("/summary.md") }
      Util.assert(packet_paths.length == 1 && summary_paths.length == 1, "SELF_ARTIFACT_POLICY_MISMATCH",
                  "materialized manifest must declare exactly one packet.json and one summary.md")

      packet_ref = @self_artifacts["packet"]
      summary_ref = @self_artifacts["summary"]
      Util.assert(packet_ref.is_a?(Hash) && summary_ref.is_a?(Hash), "SELF_ARTIFACT_SET_MISMATCH",
                  "packet and summary selfArtifacts must be digest refs")
      Util.assert(packet_ref["ref"] == packet_paths.first && summary_ref["ref"] == summary_paths.first,
                  "SELF_ARTIFACT_PATH_MISMATCH", "packet/summary refs differ from materialized manifest paths")
      stored_packet = @store.json(packet_ref)
      Util.assert(CanonicalJSON.dump(stored_packet) == CanonicalJSON.dump(@packet) &&
                  packet_ref["sha256"] == CanonicalJSON.sha256(@packet), "PACKET_SELF_OBJECT_MISMATCH",
                  "separately stored packet object differs from the accepted packet")
      summary = @store.utf8(summary_ref)
      Util.assert(!summary.empty?, "SUMMARY_SELF_OBJECT_EMPTY", "separately hashed summary.md must be non-empty")
    end

    def verify_cost_accounting!
      cost = @receipt["costAccounting"]
      Util.assert(cost.is_a?(Hash), "COST_ACCOUNTING_MISSING", "receipt costAccounting is missing")
      ancestry = cost["priorAttemptReceipts"]
      history = @runtime["attemptHistory"] || []
      Util.assert(ancestry.is_a?(Array), "RETRY_ANCESTRY_MISSING", "costAccounting.priorAttemptReceipts is missing")
      Util.assert(CanonicalJSON.dump(ancestry) == CanonicalJSON.dump(history), "RETRY_ANCESTRY_MISMATCH", "receipt retry ancestry differs from trusted host attempt history")

      attempt = @packet["attempt"]
      if attempt == 1
        Util.assert(ancestry.empty?, "RETRY_ANCESTRY_MISMATCH", "attempt 1 must have empty retry ancestry")
        Util.assert(@runtime["retryAuthorization"].nil?, "UNEXPECTED_RETRY_AUTHORIZATION",
                    "attempt 1 runtime state must not contain retryAuthorization")
      elsif attempt == 2
        Util.assert(ancestry.length == 1 && ancestry.first["attempt"] == 1, "RETRY_ANCESTRY_MISMATCH", "attempt 2 must name exactly attempt 1")
      else
        raise Reject.new("ATTEMPT_OUT_OF_RANGE", "bootstrap controller supports producer attempts 1 and 2 only", attempt)
      end

      pairs = ancestry.map { |entry| load_prior_attempt!(entry) }
      pairs.each_cons(2) do |left, right|
        Util.assert(Util.parse_time(left[1]["verifiedAt"], "prior receipt verifiedAt") <= Util.parse_time(right[0]["startedAt"], "prior packet startedAt"),
                    "RETRY_TIME_OVERLAP", "retry attempt starts before prior verification completes")
      end
      unless pairs.empty?
        Util.assert(Util.parse_time(pairs.last[1]["verifiedAt"], "prior receipt verifiedAt") <= Util.parse_time(@packet["startedAt"], "packet.startedAt"),
                    "RETRY_TIME_OVERLAP", "current retry starts before prior verification completes")
      end

      current_pair = [@packet, @receipt]
      all_pairs = pairs + [current_pair]
      all_pairs.each { |packet, receipt| verify_attempt_clock!(packet, receipt) }

      producer_digest = CanonicalJSON.sha256(@packet["telemetry"])
      Util.assert(cost["producerTelemetryDigest"] == producer_digest, "PRODUCER_TELEMETRY_DIGEST_MISMATCH", "producer telemetry digest is not reproducible")
      budget_digest = CanonicalJSON.sha256(@manifest["budgets"])
      Util.assert(cost["budgetRefDigest"] == budget_digest, "BUDGET_DIGEST_MISMATCH", "budgetRefDigest is not the materialized manifest budget digest")

      telemetries = []
      all_pairs.each do |packet, receipt|
        telemetries << packet["telemetry"]
        telemetries << receipt.dig("costAccounting", "verifierTelemetry")
      end
      telemetries.each { |telemetry| verify_telemetry!(telemetry) }
      native_available = telemetries.all? { |telemetry| telemetry["nativeTokenTelemetryAvailable"] == true }
      totals = {
        "nativeTokenTelemetryAvailable" => native_available,
        "inputTokens" => native_available ? telemetries.sum { |telemetry| telemetry["inputTokens"] } : nil,
        "cachedInputTokens" => native_available ? telemetries.sum { |telemetry| telemetry["cachedInputTokens"] } : nil,
        "outputTokens" => native_available ? telemetries.sum { |telemetry| telemetry["outputTokens"] } : nil,
        "noncachedTokens" => native_available ? telemetries.sum { |telemetry| telemetry["noncachedTokens"] } : nil,
        "toolCalls" => telemetries.sum { |telemetry| telemetry["toolCalls"] },
        "teamWallMilliseconds" => ((Util.parse_time(@receipt["verifiedAt"], "receipt.verifiedAt") -
                                    all_pairs.map { |packet, _receipt| Util.parse_time(packet["startedAt"], "packet.startedAt") }.min) * 1000).round,
        "maxVisibleContextBytes" => telemetries.map { |telemetry| telemetry["visibleContextBytes"] }.max
      }
      Util.assert(CanonicalJSON.dump(cost["teamTotals"]) == CanonicalJSON.dump(totals), "TEAM_TOTAL_MISMATCH", "receipt teamTotals do not recompute", { "expected" => totals, "actual" => cost["teamTotals"] })

      exceeded = exceeded_metrics(totals, @manifest["budgets"])
      expected_status = if exceeded.empty?
                          native_available ? "WITHIN" : "TOKEN_TELEMETRY_UNAVAILABLE"
                        else
                          "EXCEEDED"
                        end
      Util.assert(cost["budgetStatus"] == expected_status, "BUDGET_STATUS_MISMATCH", "receipt budgetStatus does not match recomputed ceilings", { "expected" => expected_status, "actual" => cost["budgetStatus"] })
      Util.assert((cost["exceededMetrics"] || []).sort == exceeded.sort, "EXCEEDED_METRIC_MISMATCH", "receipt exceededMetrics do not match recomputed ceilings", { "expected" => exceeded, "actual" => cost["exceededMetrics"] })

      {
        "budgetStatus" => expected_status,
        "teamTotals" => totals,
        "exceededMetrics" => exceeded
      }
    end

    def load_prior_attempt!(entry)
      Util.assert(entry.is_a?(Hash) && entry["attempt"].is_a?(Integer), "INVALID_RETRY_ANCESTRY", "prior attempt ancestry entry is invalid", entry)
      packet = @store.json(entry["packetRef"])
      receipt = @store.receipt(entry["receiptRef"])
      unless @self_test
        validate_with_ajv!(@schema_paths.fetch("packet"), packet, "priorPacket")
        validate_with_ajv!(@schema_paths.fetch("receipt"), receipt, "priorReceipt")
      end
      Util.assert(packet["attempt"] == entry["attempt"] && receipt["attempt"] == entry["attempt"], "RETRY_ANCESTRY_MISMATCH", "prior attempt numbers do not match ancestry")
      Util.assert(packet["nodeId"] == @packet["nodeId"] && receipt["nodeId"] == @packet["nodeId"], "RETRY_NODE_MISMATCH", "prior attempt belongs to another node")
      Util.assert(receipt["verdict"] == "ACCEPT", "PRIOR_ATTEMPT_NOT_ACCEPTED", "prior attempt receipt must be accepted")
      approval_digest = CanonicalJSON.sha256(@approval_bundle)
      Util.assert(packet["approvalBundleDigest"] == approval_digest && receipt["approvalBundleDigest"] == approval_digest,
                  "PRIOR_APPROVAL_BUNDLE_MISMATCH", "prior attempt belongs to another approval bundle")
      prior_bindings = Util.deep_copy(@effective_bindings)
      prior_bindings["ATTEMPT"] = entry["attempt"].to_s
      template = @manifest_bundle.fetch("manifests").find { |manifest| manifest["id"] == @packet["nodeId"] }
      prior_manifest = Materializer.new(prior_bindings).materialize(template)
      prior_manifest_digest = CanonicalJSON.sha256(prior_manifest)
      Util.assert(packet["runManifestDigest"] == prior_manifest_digest && receipt["runManifestDigest"] == prior_manifest_digest,
                  "PRIOR_RUN_MANIFEST_MISMATCH", "prior attempt manifest digest is not the exact prior-attempt materialization")
      Util.assert(receipt["packetDigest"] == CanonicalJSON.sha256(packet), "PRIOR_PACKET_DIGEST_MISMATCH", "prior packet digest is not reproducible")
      Util.assert(receipt["receiptDigest"] == CanonicalJSON.sha256(Util.without_key(receipt, "receiptDigest")), "PRIOR_RECEIPT_DIGEST_MISMATCH", "prior receipt digest is not reproducible")
      Util.assert(receipt["digestScope"] == "RFC8785_CANONICAL_JSON_WITHOUT_RECEIPT_DIGEST", "PRIOR_RECEIPT_DIGEST_SCOPE_MISMATCH",
                  "prior receipt digestScope is invalid")
      Util.assert(receipt["packetOutcome"] == packet["outcome"] && receipt["packetBranchCode"] == packet["branchCode"],
                  "PRIOR_PACKET_RECEIPT_OUTCOME_MISMATCH", "prior receipt outcome or branch differs from its packet")
      Util.assert(@manifest["allowedOutcomes"].include?(packet["outcome"]) && @manifest["branchCodes"].include?(packet["branchCode"]),
                  "PRIOR_OUTCOME_NOT_REGISTERED", "prior attempt outcome or branch is not registered")
      Util.assert(Util.exact_set?(packet["hypothesisIds"], prior_manifest["hypothesisIds"]), "PRIOR_HYPOTHESIS_SET_MISMATCH",
                  "prior packet hypothesis set differs from its materialized manifest")
      producer_session = packet.dig("producer", "sessionId")
      Util.assert(receipt["producerSessionId"] == producer_session && receipt.dig("verifier", "sessionId") != producer_session &&
                  receipt["independenceAttestation"] == true, "PRIOR_SESSION_INDEPENDENCE_VIOLATION",
                  "prior producer/verifier identity is invalid")
      prior_checks = receipt["checks"] || []
      Util.assert(prior_checks.map { |check| check["checkId"] } == prior_manifest["requiredCheckIds"] &&
                  prior_checks.all? { |check| check["result"] == "PASS" }, "PRIOR_MANDATORY_CHECK_FAILED",
                  "prior accepted receipt lacks the exact mandatory PASS set")
      Util.assert(receipt.dig("costAccounting", "priorAttemptReceipts") == [], "PRIOR_RETRY_ANCESTRY_MISMATCH",
                  "attempt 1 must not contain retry ancestry")
      Util.assert(receipt.dig("costAccounting", "producerTelemetryDigest") == CanonicalJSON.sha256(packet["telemetry"]), "PRIOR_TELEMETRY_DIGEST_MISMATCH", "prior producer telemetry digest is not reproducible")
      prior_totals = recompute_single_attempt_totals(packet, receipt)
      prior_cost = receipt["costAccounting"]
      Util.assert(CanonicalJSON.dump(prior_cost["teamTotals"]) == CanonicalJSON.dump(prior_totals), "PRIOR_TEAM_TOTAL_MISMATCH",
                  "prior receipt teamTotals do not recompute")
      prior_exceeded = exceeded_metrics(prior_totals, prior_manifest["budgets"])
      prior_status = if prior_exceeded.empty?
                       prior_totals["nativeTokenTelemetryAvailable"] ? "WITHIN" : "TOKEN_TELEMETRY_UNAVAILABLE"
                     else
                       "EXCEEDED"
                     end
      Util.assert(prior_cost["budgetStatus"] == prior_status && (prior_cost["exceededMetrics"] || []).sort == prior_exceeded.sort,
                  "PRIOR_BUDGET_STATUS_MISMATCH", "prior receipt budget status does not recompute")
      retryable_codes = prior_manifest["retryableGenericBranchCodes"]
      Util.assert(retryable_codes.is_a?(Array) && retryable_codes.length == retryable_codes.uniq.length,
                  "INVALID_RETRY_BRANCH_POLICY", "prior manifest retryableGenericBranchCodes must be an exact set")
      retryable = GENERIC_OUTCOMES.include?(packet["outcome"]) &&
                  %w[WITHIN TOKEN_TELEMETRY_UNAVAILABLE].include?(prior_status) &&
                  retryable_codes.include?(packet["branchCode"]) &&
                  !%w[NONE BUDGET_EXCEEDED].include?(packet["branchCode"])
      Util.assert(retryable, "PRIOR_ATTEMPT_NOT_RETRYABLE",
                  "attempt 2 requires an accepted generic attempt 1 on a manifest-authorized retry branch with remaining budget")
      expected_authorization = RetryAuthorization.build(entry, packet, receipt, prior_manifest)
      Util.assert(CanonicalJSON.dump(@runtime["retryAuthorization"]) == CanonicalJSON.dump(expected_authorization),
                  "RETRY_AUTHORIZATION_MISMATCH", "trusted runtime retryAuthorization is missing or not reproducible")
      verify_prior_attempt_closure!(entry, packet, receipt, prior_manifest)
      [packet, receipt]
    end

    def verify_prior_attempt_closure!(entry, packet, receipt, manifest)
      approved_sources = @approval_bundle.dig("approvalSubject", "sources") || []
      approved_keys = canonical_source_set!(approved_sources, "INVALID_APPROVED_SOURCE_SET", "approvalSubject.sources")
      runtime_keys = canonical_source_set!(@runtime["currentSourceDigests"] || [], "INVALID_RUNTIME_SOURCE_SET",
                                           "runtimeState.currentSourceDigests")
      prior_keys = canonical_source_set!(packet["sourceDigests"] || [], "INVALID_PRIOR_SOURCE_SET",
                                         "prior packet sourceDigests")
      Util.assert(runtime_keys == approved_keys, "RUNTIME_APPROVED_SOURCE_SET_MISMATCH",
                  "trusted runtime sources differ from the human-approved S0-S5 source set")
      Util.assert(prior_keys == approved_keys, "PRIOR_SOURCE_APPROVAL_SET_MISMATCH",
                  "prior packet sources differ from the human-approved S0-S5 source set")
      Util.assert(digest_ref_arrays_equal?(packet["parentReceiptDigests"], @runtime["expectedParentReceiptDigests"] || []),
                  "PRIOR_PARENT_RECEIPT_SET_MISMATCH", "prior packet predecessor set differs from trusted runtime state")
      (packet["sourceDigests"] || []).each { |ref| @store.resolve(ref) }
      (packet["parentReceiptDigests"] || []).each { |ref| @store.resolve(ref) }

      policy = ArtifactPolicy.new(manifest, packet["outcome"], packet["branchCode"])
      artifacts = packet["artifacts"] || []
      paths = artifacts.map { |artifact| artifact["path"] }
      Util.assert(paths.all? { |path| path.is_a?(String) && !path.empty? } && paths.length == paths.uniq.length,
                  "PRIOR_PACKET_ARTIFACT_SET_INVALID", "prior packet artifact paths must be unique non-empty strings")
      missing = policy.required_paths - paths
      Util.assert(missing.empty?, "MISSING_GENERIC_DIAGNOSTIC", "prior generic packet lacks required genericOutcomeArtifacts", missing)
      success_policy = ArtifactPolicy.new(manifest.merge("genericOutcomeArtifacts" => manifest["outputArtifacts"]), "FAILURE", packet["branchCode"])
      declared = (policy.allowed_paths + success_policy.allowed_paths).uniq
      extra = paths - declared
      Util.assert(extra.empty?, "UNDECLARED_PACKET_ARTIFACT", "prior generic packet contains undeclared artifacts", extra)
      preexisting = @runtime["preexistingSuccessArtifacts"] || []
      (paths & success_policy.allowed_paths).each do |path|
        artifact = artifacts.find { |candidate| candidate["path"] == path }
        allowed = preexisting.any? { |ref| ref["ref"] == path && ref["sha256"] == artifact["sha256"] }
        Util.assert(allowed, "UNPROVEN_PREEXISTING_ARTIFACT", "prior generic success-artifact subset was not already immutable", path)
      end
      Util.assert(packet["artifactSetDigest"] == CanonicalJSON.sha256(paths.sort), "ARTIFACT_SET_DIGEST_MISMATCH",
                  "prior packet artifactSetDigest is not the exact sorted actual path digest")

      artifact_index = {}
      artifacts.each do |artifact|
        ref = { "ref" => artifact["path"], "sha256" => artifact["sha256"] }
        @store.resolve(ref)
        artifact_index[artifact["path"]] = artifact["sha256"]
      end
      evidence_refs = []
      (packet["claims"] || []).each { |claim| evidence_refs.concat(claim["evidenceRefs"] || []) }
      evidence_refs.concat(packet.dig("evidenceDelta", "artifactRefs") || [])
      (receipt["checks"] || []).each { |check| evidence_refs << check["evidenceRef"] }
      authorized = @runtime["authorizedEvidenceRefs"] || []
      evidence_refs.each do |ref|
        name, digest = Util.digest_ref_key(ref)
        if artifact_index.key?(name)
          Util.assert(artifact_index[name] == digest, "EVIDENCE_REF_DIGEST_MISMATCH",
                      "prior evidence ref conflicts with a packet artifact", ref)
        else
          allowed = authorized.any? { |candidate| candidate["ref"] == name && candidate["sha256"] == digest }
          Util.assert(allowed, "UNAUTHORIZED_EVIDENCE_REF", "prior evidence ref is not local or authorized", ref)
        end
        @store.resolve(ref)
      end

      packet_available = packet.dig("telemetry", "nativeTokenTelemetryAvailable")
      expected_eligibility = packet_available ? "AVAILABLE" : "UNAVAILABLE"
      Util.assert(packet.dig("metricEligibility", "nativeTokens") == expected_eligibility, "METRIC_ELIGIBILITY_MISMATCH",
                  "prior packet metric eligibility differs from producer telemetry")
      unless packet_available
        (packet["claims"] || []).each do |claim|
          Util.assert(!(claim["domains"] || []).include?("TOKEN"), "TOKEN_DOMAIN_FORBIDDEN",
                      "prior claim uses TOKEN domain while telemetry is unavailable", claim["claimId"])
        end
        Util.assert(!(packet.dig("evidenceDelta", "domains") || []).include?("TOKEN"), "TOKEN_DOMAIN_FORBIDDEN",
                    "prior evidence delta uses TOKEN domain while telemetry is unavailable")
      end

      self_records = policy.records.select { |record| record["selfReferential"] }
      packet_path = self_records.map { |record| record["path"] }.find { |path| path.end_with?("/packet.json") }
      summary_path = self_records.map { |record| record["path"] }.find { |path| path.end_with?("/summary.md") }
      Util.assert(packet_path && summary_path, "SELF_ARTIFACT_POLICY_MISMATCH", "prior manifest lacks packet/summary self artifacts")
      packet_self_ref = entry["packetSelfRef"]
      summary_self_ref = entry["summarySelfRef"]
      Util.assert(packet_self_ref.is_a?(Hash) && summary_self_ref.is_a?(Hash) &&
                  packet_self_ref["ref"] == packet_path && summary_self_ref["ref"] == summary_path,
                  "PRIOR_SELF_ARTIFACT_REF_MISMATCH", "prior ancestry self refs differ from materialized packet/summary paths")
      stored_packet = @store.json(packet_self_ref)
      Util.assert(CanonicalJSON.dump(stored_packet) == CanonicalJSON.dump(packet) &&
                  packet_self_ref["sha256"] == CanonicalJSON.sha256(packet), "PACKET_SELF_OBJECT_MISMATCH",
                  "prior separately stored packet differs from the ancestry packet")
      Util.assert(!@store.utf8(summary_self_ref).empty?, "SUMMARY_SELF_OBJECT_EMPTY",
                  "prior separately hashed summary.md must be non-empty")
    end

    def recompute_single_attempt_totals(packet, receipt)
      telemetries = [packet["telemetry"], receipt.dig("costAccounting", "verifierTelemetry")]
      telemetries.each { |telemetry| verify_telemetry!(telemetry) }
      native_available = telemetries.all? { |telemetry| telemetry["nativeTokenTelemetryAvailable"] == true }
      {
        "nativeTokenTelemetryAvailable" => native_available,
        "inputTokens" => native_available ? telemetries.sum { |telemetry| telemetry["inputTokens"] } : nil,
        "cachedInputTokens" => native_available ? telemetries.sum { |telemetry| telemetry["cachedInputTokens"] } : nil,
        "outputTokens" => native_available ? telemetries.sum { |telemetry| telemetry["outputTokens"] } : nil,
        "noncachedTokens" => native_available ? telemetries.sum { |telemetry| telemetry["noncachedTokens"] } : nil,
        "toolCalls" => telemetries.sum { |telemetry| telemetry["toolCalls"] },
        "teamWallMilliseconds" => ((Util.parse_time(receipt["verifiedAt"], "prior receipt.verifiedAt") -
                                    Util.parse_time(packet["startedAt"], "prior packet.startedAt")) * 1000).round,
        "maxVisibleContextBytes" => telemetries.map { |telemetry| telemetry["visibleContextBytes"] }.max
      }
    end

    def verify_attempt_clock!(packet, receipt)
      started = Util.parse_time(packet["startedAt"], "packet.startedAt")
      produced = Util.parse_time(packet["producerCompletedAt"], "packet.producerCompletedAt")
      verified = Util.parse_time(receipt["verifiedAt"], "receipt.verifiedAt")
      Util.assert(started <= produced && produced <= verified, "NON_MONOTONIC_EVENT_CLOCK", "attempt timestamps are not monotonic")
    end

    def verify_telemetry!(telemetry)
      Util.assert(telemetry.is_a?(Hash), "TELEMETRY_MISSING", "producer or verifier telemetry is missing")
      available = telemetry["nativeTokenTelemetryAvailable"]
      Util.assert(available == true || available == false, "TOKEN_AVAILABILITY_INVALID", "nativeTokenTelemetryAvailable must be boolean")
      if available
        TOKEN_FIELDS.each { |field| Util.assert(telemetry[field].is_a?(Integer) && telemetry[field] >= 0, "TOKEN_TELEMETRY_INVALID", "#{field} must be a nonnegative integer") }
        Util.assert(telemetry["cachedInputTokens"] <= telemetry["inputTokens"], "TOKEN_TELEMETRY_FORMULA_MISMATCH", "cached input tokens exceed input tokens")
        expected = telemetry["inputTokens"] - telemetry["cachedInputTokens"] + telemetry["outputTokens"]
        Util.assert(telemetry["noncachedTokens"] == expected, "TOKEN_TELEMETRY_FORMULA_MISMATCH", "noncachedTokens must equal input-cached+output", { "expected" => expected, "actual" => telemetry["noncachedTokens"] })
      else
        TOKEN_FIELDS.each { |field| Util.assert(telemetry[field].nil?, "TOKEN_TELEMETRY_INVALID", "#{field} must be null when native telemetry is unavailable") }
      end
      %w[toolCalls wallMilliseconds visibleContextBytes].each do |field|
        Util.assert(telemetry[field].is_a?(Integer) && telemetry[field] >= 0, "TELEMETRY_INVALID", "#{field} must be a nonnegative integer")
      end
    end

    def exceeded_metrics(totals, budgets)
      exceeded = []
      if totals["nativeTokenTelemetryAvailable"]
        exceeded << "NONCACHED_TOKENS" if totals["noncachedTokens"] > budgets["noncachedTokenCeiling"]
        exceeded << "OUTPUT_TOKENS" if totals["outputTokens"] > budgets["outputTokenCeiling"]
      end
      exceeded << "TOOL_CALLS" if totals["toolCalls"] > budgets["toolCallCeiling"]
      exceeded << "WALL_MILLISECONDS" if totals["teamWallMilliseconds"] > budgets["wallMinutesCeiling"] * 60_000
      exceeded << "VISIBLE_CONTEXT_BYTES" if totals["maxVisibleContextBytes"] > budgets["perTurnVisibleContextBytes"]
      exceeded
    end

    def verify_metric_eligibility!(cost)
      eligibility = @packet["metricEligibility"] || {}
      packet_available = @packet.dig("telemetry", "nativeTokenTelemetryAvailable")
      expected = packet_available ? "AVAILABLE" : "UNAVAILABLE"
      Util.assert(eligibility["nativeTokens"] == expected, "METRIC_ELIGIBILITY_MISMATCH",
                  "metricEligibility.nativeTokens differs from immutable packet producer telemetry")
      if @packet["nodeId"] == "R02" && @packet["outcome"] == "SUCCESS"
        expected_branch = packet_available ? "NONE" : "TOKEN_TELEMETRY_UNAVAILABLE"
        Util.assert(@packet["branchCode"] == expected_branch, "R02_TOKEN_BRANCH_MISMATCH", "R02 SUCCESS branch does not match producer native-token availability")
      end
      return if packet_available

      (@packet["claims"] || []).each do |claim|
        Util.assert(!(claim["domains"] || []).include?("TOKEN"), "TOKEN_DOMAIN_FORBIDDEN", "claim uses TOKEN domain while native telemetry is unavailable", claim["claimId"])
      end
      domains = (@packet.dig("evidenceDelta", "domains") || [])
      Util.assert(!domains.include?("TOKEN"), "TOKEN_DOMAIN_FORBIDDEN", "evidence delta uses TOKEN domain while native telemetry is unavailable")
    end

    def verify_gk_wave!
      wave = @runtime["gkWave"]
      Util.assert(wave.is_a?(Hash), "GK_WAVE_STATE_MISSING", "trusted GK wave state is required")
      Util.assert(wave["scope"] == "FOUNDATION" && wave["quiescent"] == true,
                  "GK_WAVE_NOT_QUIESCENT", "GK bootstrap wave must be FOUNDATION and quiescent")
      Util.assert(wave["normalContinuationReachable"] == false, "GK_CONTINUATION_STILL_REACHABLE", "GK bootstrap profile cannot activate while normal continuation is reachable")
      parents = wave["exhaustedParents"]
      Util.assert(parents.is_a?(Array) && !parents.empty?, "GK_EXHAUSTED_PARENT_SET_EMPTY", "GK requires at least one exhausted bootstrap parent")
      eligible = %w[R01 R02 R03]
      receipt_refs = []
      parent_nodes = []
      parents.each do |parent|
        Util.assert(eligible.include?(parent["nodeId"]), "GK_INELIGIBLE_PARENT", "GK v0 parent is outside R01/R02/R03", parent["nodeId"])
        Util.assert(parent["accepted"] == true && parent["exhausted"] == true && parent["reachableContinuation"] == false,
                    "GK_PARENT_NOT_EXHAUSTED", "GK parent is not an accepted exhausted input", parent)
        Util.assert(GENERIC_OUTCOMES.include?(parent["effectiveOutcome"]), "GK_PARENT_NOT_GENERIC", "GK exhausted parent must have a generic effective outcome", parent)
        packet = @store.json(parent["packetRef"])
        receipt = @store.json(parent["receiptRef"])
        Util.assert(packet["nodeId"] == parent["nodeId"] && receipt["nodeId"] == parent["nodeId"], "GK_PARENT_NODE_MISMATCH", "GK parent objects belong to another node")
        Util.assert(receipt["verdict"] == "ACCEPT" && receipt["packetDigest"] == CanonicalJSON.sha256(packet), "GK_PARENT_NOT_ACCEPTED", "GK parent packet/receipt is not accepted")
        Util.assert(receipt["receiptDigest"] == parent.dig("receiptRef", "sha256"), "GK_PARENT_RECEIPT_DIGEST_MISMATCH", "GK parent receipt digest differs from wave ref")
        receipt_refs << parent["receiptRef"]
        parent_nodes << parent["nodeId"]
      end
      Util.assert(parent_nodes.length == parent_nodes.uniq.length, "GK_DUPLICATE_PARENT", "GK exhausted parent nodes must be unique")
      expected_sorted = receipt_refs.sort_by { |ref| [ref["ref"], ref["sha256"]] }
      Util.assert(CanonicalJSON.dump(@packet["parentReceiptDigests"]) == CanonicalJSON.dump(expected_sorted), "GK_PARENT_RECEIPT_SET_MISMATCH", "GK packet does not contain the exact sorted quiescent parent set")
      Util.assert(@packet["outcome"] == "SUCCESS" && @packet["branchCode"] == "INCONCLUSIVE_FOUNDATION",
                  "GK_BOOTSTRAP_OUTPUT_INVALID", "GK v0 can emit only SUCCESS+INCONCLUSIVE_FOUNDATION")
      if @effective_bindings.key?("EXHAUSTED_SOURCE")
        bound = @effective_bindings["EXHAUSTED_SOURCE"]
        bound = [bound] unless bound.is_a?(Array)
        Util.assert(bound.sort == parent_nodes.sort, "GK_EXHAUSTED_SOURCE_BINDING_MISMATCH", "EXHAUSTED_SOURCE binding differs from exact wave parent nodes")
      end
    end

    def effective_projection(cost)
      if cost["budgetStatus"] == "EXCEEDED"
        { "outcome" => "NO_PROGRESS", "branchCode" => "BUDGET_EXCEEDED", "retryState" => "EXHAUSTED" }
      else
        { "outcome" => @packet["outcome"], "branchCode" => @packet["branchCode"], "retryState" => @runtime["retryState"] || "NOT_APPLICABLE" }
      end
    end

    def derive_effective_edges(effective)
      dot_digest = @approval_bundle.dig("approvalSubject", "authoritativeDot", "sha256")
      Util.assert(@runtime["dotDigest"] == dot_digest, "DOT_DIGEST_MISMATCH", "trusted runtime edge registry is not tied to approved DOT")
      Util.assert(@runtime["allDigestsCurrent"] == true, "STALE_DIGEST", "trusted runtime reports stale digests")
      runtime_registry = @runtime["edgeRegistry"]
      Util.assert(runtime_registry.is_a?(Array), "EDGE_REGISTRY_MISSING", "trusted runtime edge registry is missing")
      approved_registry = DotEdgeRegistry.for_source(@approved_dot, @packet["nodeId"])
      runtime_sorted = runtime_registry.sort_by { |edge| edge.is_a?(Hash) ? edge["id"].to_s : "" }
      Util.assert(runtime_sorted.length == approved_registry.length, "EDGE_REGISTRY_DOT_MISMATCH",
                  "runtime edge registry must contain every and only approved DOT row for this source")
      registry = approved_registry.map do |approved|
        runtime_edge = runtime_sorted.find { |edge| edge.is_a?(Hash) && edge["id"] == approved["id"] }
        Util.assert(!runtime_edge.nil? && runtime_edge.keys.sort == DotEdgeRegistry::AUTHORIZATION_FIELDS.sort,
                    "EDGE_REGISTRY_DOT_MISMATCH", "runtime edge row shape differs from approved DOT", approved["id"])
        static_fields = DotEdgeRegistry::AUTHORIZATION_FIELDS - ["gatePermitted"]
        static_fields.each do |field|
          Util.assert(runtime_edge[field] == approved[field], "EDGE_REGISTRY_DOT_MISMATCH",
                      "runtime edge #{approved['id']} changes approved DOT field #{field}",
                      { "approved" => approved[field], "runtime" => runtime_edge[field] })
        end
        Util.assert(runtime_edge["gatePermitted"] == true || runtime_edge["gatePermitted"] == false,
                    "EDGE_REGISTRY_DOT_MISMATCH", "runtime gatePermitted must be boolean", approved["id"])
        approved.merge("gatePermitted" => runtime_edge["gatePermitted"])
      end
      return [] unless @receipt["verdict"] == "ACCEPT"

      edges = registry.select do |edge|
        next false unless edge["source"] == @packet["nodeId"]
        next false unless (edge["acceptedOutcomes"] || []).include?(effective["outcome"])
        branch_match = if edge["branchMatch"] == "ANY_SOURCE_REGISTERED_CODE"
                         GENERIC_OUTCOMES.include?(effective["outcome"]) && @manifest["branchCodes"].include?(effective["branchCode"])
                       else
                         (edge["acceptedBranchCodes"] || []).include?(effective["branchCode"])
                       end
        next false unless branch_match
        required_retry = edge["retryState"] || "NOT_APPLICABLE"
        next false unless required_retry == "NOT_APPLICABLE" || required_retry == effective["retryState"]
        next false if edge["gatePermitted"] == false
        true
      end
      ids = edges.map { |edge| edge["id"] }
      Util.assert(ids.all? { |id| id.is_a?(String) && !id.empty? } && ids.length == ids.uniq.length, "INVALID_EDGE_REGISTRY", "eligible edge IDs must be unique non-empty strings")
      if @packet["nodeId"] == "GK"
        Util.assert(ids == ["GK->GF0"], "GK_IMPLEMENTATION_UNLOCK", "GK bootstrap profile must expose exactly GK->GF0", ids)
      end
      ids.sort
    end

    def digest_ref_arrays_equal?(left, right)
      return false unless left.is_a?(Array) && right.is_a?(Array)
      left_keys = left.map { |ref| Util.digest_ref_key(ref) }
      right_keys = right.map { |ref| Util.digest_ref_key(ref) }
      left_keys.length == left_keys.uniq.length && right_keys.length == right_keys.uniq.length && left_keys.sort == right_keys.sort
    end

    def canonical_source_set!(refs, code, label)
      Util.assert(refs.is_a?(Array) && refs.length == APPROVED_SOURCE_ROLES.length, code,
                  "#{label} must contain exactly the six approved sources", refs)
      keys = refs.map do |ref|
        Util.assert(ref.is_a?(Hash) && ref.keys.sort == %w[ref role sha256], code,
                    "#{label} entries must contain exactly role, ref, and sha256", ref)
        role = ref["role"]
        name = ref["ref"]
        digest = ref["sha256"]
        Util.assert(APPROVED_SOURCE_ROLES.include?(role), code, "#{label} contains an unknown source role", role)
        Util.assert(name.is_a?(String) && !name.empty?, code, "#{label} contains an invalid source ref", ref)
        Util.assert(digest.is_a?(String) && SHA256_PATTERN.match?(digest), code,
                    "#{label} contains an invalid source digest", ref)
        [role, name, digest]
      end
      roles = keys.map(&:first)
      names = keys.map { |key| key[1] }
      Util.assert(roles.sort == APPROVED_SOURCE_ROLES && roles.length == roles.uniq.length, code,
                  "#{label} must contain each S0-S5 role exactly once", roles)
      Util.assert(names.length == names.uniq.length && keys.length == keys.uniq.length, code,
                  "#{label} source refs must be unique", keys)
      keys.sort
    end

    def exact_string_set?(left, right)
      left.is_a?(Array) && right.is_a?(Array) && left.length == left.uniq.length && right.length == right.uniq.length && left.sort == right.sort
    end
  end

  class SelfTest
    PLAN_DIR = File.expand_path(__dir__)
    DOT_PATH = File.join(PLAN_DIR, "2026-08-08-codeclew-cumulative-evidence-graph.dot")
    MANIFEST_PATH = File.join(PLAN_DIR, "2026-08-08-codeclew-bootstrap-manifests-v0.json")
    FIXTURE_SPEC_PATH = File.join(PLAN_DIR, "2026-08-08-codeclew-bootstrap-contract-fixtures-v0.json")
    NODE_SCHEMA_PATH = File.join(PLAN_DIR, "2026-08-08-codeclew-node-contract-v0.schema.json")
    PACKET_SCHEMA_PATH = File.join(PLAN_DIR, "2026-08-08-codeclew-evidence-packet-v0.schema.json")
    RECEIPT_SCHEMA_PATH = File.join(PLAN_DIR, "2026-08-08-codeclew-verification-receipt-v0.schema.json")
    APPROVAL_SCHEMA_PATH = File.join(PLAN_DIR, "2026-08-08-codeclew-approval-bundle-v0.schema.json")

    def initialize
      @passed = []
      @failed = []
    end

    def run
      test("compact fixture spec names the exact dynamic controller cases") { test_fixture_spec }
      test("AJV accepts generated approval/manifests/packet/receipt") { test_ajv }
      test("AJV rejects declared invalid approval, manifest, packet, and receipt mutations") { test_ajv_rejections }
      test("recursive placeholder materialization expands arrays and rejects cycles") { test_materializer }
      test("full A00 -> R01 -> R02 success chain receives only authoritative continuation edges") { test_baseline }
      test("TEST_ONLY cannot enter normal runtime verification") { test_test_only_boundary }
      test("approval subject tampering is rejected") { test_approval_tamper }
      test("current-session approval is USER-role and digest bound") { test_session_approval_binding }
      test("runtime manifest substitution is rejected against the approved artifact") { test_manifest_substitution }
      test("executing controller bytes must match the human-approved controller artifact") { test_controller_digest_binding }
      test("current and prior packets must carry exactly the human-approved S0-S5 source set") { test_approved_source_closure }
      test("duplicate manifest IDs are rejected even when the manifest artifact is approved") { test_duplicate_manifest_id }
      test("ACCEPT cannot contain a failed mandatory check") { test_accept_with_failed_check }
      test("missing, wrong, and duplicate mandatory check IDs are rejected") { test_required_check_id_mutations }
      test("packet hypotheses must equal the manifest hypothesis set") { test_hypothesis_set }
      test("producer and verifier sessions must be independent") { test_session_independence }
      test("producer and verifier timestamps must be monotonic") { test_inverted_timestamps }
      test("SUCCESS requires the exact materialized success artifact set") { test_success_artifact_policy }
      test("generic outcome requires the exact failure.json path and permits only proven success subset") { test_generic_artifact_policy }
      test("packet.json and summary.md are separately path-bound, resolved, and hashed") { test_self_artifacts }
      test("evidence references must resolve and match packet artifact hashes") { test_evidence_reference_mutations }
      test("unavailable native telemetry cannot carry TOKEN-domain evidence") { test_token_domain }
      test("available native telemetry requires AVAILABLE metric eligibility") { test_available_metric_eligibility }
      test("packet-local token eligibility remains AVAILABLE when only verifier telemetry is unavailable") { test_mixed_token_availability }
      test("over-budget raw SUCCESS cannot continue to K01") { test_over_budget }
      test("noncached token undercount is rejected") { test_token_formula }
      test("attempt 2 requires exact accepted attempt-1 ancestry and cumulative wall/cost") { test_retry_ancestry }
      test("attempt 2 rejects a prior receipt with a mismatched self-digest") { test_prior_receipt_digest }
      test("attempt 2 rejects prior artifact closure and ancestry self-ref tampering") { test_prior_artifact_closure }
      test("producer proposedNextEdges never authorize an edge") { test_effective_edges_authoritative }
      test("GK accepts only the exact quiescent R02/R03 exhausted set, branch, and GF0 edge") { test_gk }
      test("normal Codex mode accepts session approval and rejects runtime authority collisions") { test_codex_read_only_mode }

      {
        "suite" => "codeclew-bootstrap-controller-v0",
        "status" => @failed.empty? ? "PASS" : "FAIL",
        "passed" => @passed,
        "failed" => @failed,
        "assumptions" => [
          "Runtime --verify requires AJV schema paths pinned by the approval subject.",
          "CURRENT_CODEX_SESSION is an explicit local human gate checked from the task transcript by the orchestrator; it is not third-party authentication.",
          "artifactSetDigest is SHA-256 of RFC8785 canonical sorted exact actual packet artifact paths.",
          "priorAttemptReceipts resolves attempt-1 packet and receipt objects; receipt refs use the receipt digest scope.",
          "runtimeState requires two independent read-only evidence/Git observations and remains subject to controller derivation checks against approved artifacts and DOT."
        ]
      }
    end

    private

    def test(name)
      yield
      @passed << name
    rescue StandardError => error
      @failed << {
        "name" => name,
        "error" => error.class.name,
        "code" => error.respond_to?(:code) ? error.code : nil,
        "message" => error.message,
        "backtrace" => (error.backtrace || []).first(3)
      }
    end

    def expect_reject(code = nil)
      begin
        yield
      rescue Reject => error
        Util.assert(code.nil? || error.code == code, "SELF_TEST_WRONG_REJECTION", "expected #{code}, got #{error.code}")
        return error
      end
      raise Reject.new("SELF_TEST_EXPECTED_REJECTION", "mutation was unexpectedly accepted", code)
    end

    def controller
      Controller.new(self_test: true)
    end

    def clone(value)
      Util.deep_copy(value)
    end

    def digest_ref(ref, sha256)
      { "ref" => ref, "sha256" => sha256 }
    end

    def role_ref(role, ref, sha256)
      { "role" => role, "ref" => ref, "sha256" => sha256 }
    end

    def add_object(objects, ref, content, encoding = "UTF8")
      digest = case encoding
               when "UTF8"
                 Digest::SHA256.hexdigest(content.to_s)
               when "RFC8785_JSON"
                 CanonicalJSON.sha256(content)
               when "RECEIPT_WITHOUT_RECEIPT_DIGEST"
                 CanonicalJSON.sha256(Util.without_key(content, "receiptDigest"))
               else
                 raise "unsupported self-test encoding #{encoding}"
               end
      objects.reject! { |entry| entry["ref"] == ref }
      objects << { "ref" => ref, "sha256" => digest, "encoding" => encoding, "content" => clone(content) }
      digest_ref(ref, digest)
    end

    def observed_message(message_id, text)
      {
        "threadId" => "self-test-thread",
        "turnId" => "self-test-turn",
        "messageId" => message_id,
        "authorRole" => "USER",
        "messageText" => text,
        "messageTextDigest" => Digest::SHA256.hexdigest(text)
      }
    end

    def codex_observation(session_id, amendment_message, approval_message)
      observation = {
        "schemaVersion" => "codeclew-codex-observation/1",
        "provenance" => "CODEX_READ_ONLY_THREAD_OBSERVATION",
        "observerSessionId" => session_id,
        "sourceTool" => "CODEX_READ_THREAD",
        "amendmentAuthorization" => clone(amendment_message),
        "advanceApproval" => clone(approval_message),
        "observedAt" => "2026-08-08T00:00:00Z",
        "decision" => "OBSERVED_EXACT_THREAD_EVENTS"
      }
      observation["receiptDigest"] = CanonicalJSON.sha256(observation)
      observation
    end

    def scope_conformance(subject_digest, base_plan_digest, amendment_digest, session_id = "approval-observer-a")
      conformance = {
        "schemaVersion" => "codeclew-a00-scope-conformance/1",
        "verifierSessionId" => session_id,
        "decision" => "SCOPE_CONFORMANCE_ACCEPT",
        "allowedDeltaId" => "A00_PROVENANCE_AND_BOOTSTRAP_ONLY",
        "basePlanDigest" => base_plan_digest,
        "amendmentProposalDigest" => amendment_digest,
        "subjectDigest" => subject_digest,
        "blockingFindings" => 0,
        "verifiedAt" => "2026-08-08T00:00:00Z"
      }
      conformance["receiptDigest"] = CanonicalJSON.sha256(conformance)
      conformance
    end

    def runtime_observation(session_id, state_digest)
      observation = {
        "observerSessionId" => session_id,
        "source" => "READ_ONLY_EVIDENCE_AND_GIT_STATE",
        "decision" => "OBSERVED_EXACT_RUNTIME_STATE",
        "stateDigest" => state_digest,
        "observedAt" => "2026-08-08T00:00:00Z"
      }
      observation["receiptDigest"] = CanonicalJSON.sha256(observation)
      observation
    end

    def attest_runtime!(case_data)
      digest = CanonicalJSON.sha256(case_data["runtimeState"])
      case_data["runtimeAttestation"] = {
        "provenance" => "INDEPENDENT_RUNTIME_OBSERVATION_V1",
        "stateDigest" => digest,
        "observations" => [
          runtime_observation("runtime-observer-a", digest),
          runtime_observation("runtime-observer-b", digest)
        ]
      }
    end

    def reseal!(case_data)
      packet = case_data["packet"]
      receipt = case_data["receipt"]
      receipt["approvalBundleDigest"] = packet["approvalBundleDigest"]
      receipt["runManifestDigest"] = packet["runManifestDigest"]
      receipt["nodeId"] = packet["nodeId"]
      receipt["attempt"] = packet["attempt"]
      receipt["producerSessionId"] = packet.dig("producer", "sessionId")
      receipt["packetOutcome"] = packet["outcome"]
      receipt["packetBranchCode"] = packet["branchCode"]
      receipt["costAccounting"]["producerTelemetryDigest"] = CanonicalJSON.sha256(packet["telemetry"])
      receipt["packetDigest"] = CanonicalJSON.sha256(packet)
      receipt["receiptDigest"] = CanonicalJSON.sha256(Util.without_key(receipt, "receiptDigest"))
      sync_self_artifacts!(case_data)
    end

    def build_approval(objects)
      make_ref = lambda do |role, ref, content|
        stored = add_object(objects, ref, content, "UTF8")
        role_ref(role, stored["ref"], stored["sha256"])
      end
      plan = make_ref.call("PLAN", "approved/plan.md", "synthetic approved plan")
      dot = make_ref.call("AUTHORITATIVE_DOT", "approved/graph.dot", File.read(DOT_PATH))
      bootstrap_roles = %w[
        NODE_CONTRACT_SCHEMA BOOTSTRAP_MANIFESTS EVIDENCE_PACKET_SCHEMA
        VERIFICATION_RECEIPT_SCHEMA BOOTSTRAP_FIXTURES TERMINAL_SYNTHESIS
        APPROVAL_BUNDLE_SCHEMA BOOTSTRAP_CONTROLLER
      ]
      bootstrap_files = {
        "NODE_CONTRACT_SCHEMA" => NODE_SCHEMA_PATH,
        "BOOTSTRAP_MANIFESTS" => MANIFEST_PATH,
        "EVIDENCE_PACKET_SCHEMA" => PACKET_SCHEMA_PATH,
        "VERIFICATION_RECEIPT_SCHEMA" => RECEIPT_SCHEMA_PATH,
        "BOOTSTRAP_FIXTURES" => FIXTURE_SPEC_PATH,
        "TERMINAL_SYNTHESIS" => File.join(PLAN_DIR, "2026-08-08-codeclew-terminal-synthesis-v0.json"),
        "APPROVAL_BUNDLE_SCHEMA" => APPROVAL_SCHEMA_PATH,
        "BOOTSTRAP_CONTROLLER" => File.expand_path(__FILE__)
      }
      bootstrap = bootstrap_roles.map do |role|
        content = File.read(bootstrap_files.fetch(role))
        make_ref.call(role, "approved/#{role.downcase}", content)
      end
      report = make_ref.call("PLANNING_VERIFICATION_REPORT", "approved/planning-verification.json", "synthetic planning verification")
      sources = (0..5).map { |index| make_ref.call("S#{index}", "approved/S#{index}", "synthetic source S#{index}") }
      approval_message = observed_message("self-test-message:approval", "approve cumulative plan\n")
      subject = {
        "planStatus" => "PROPOSED_AWAITING_HUMAN_APPROVAL",
        "plan" => plan,
        "authoritativeDot" => dot,
        "bootstrapArtifacts" => bootstrap,
        "planningVerificationReport" => report,
        "sources" => sources
      }
      subject_digest = CanonicalJSON.sha256(subject)
      decision = {
        "schemaVersion" => "codeclew-human-decision/0",
        "decision" => "HUMAN_APPROVED",
        "subjectDigest" => subject_digest,
        "actor" => { "type" => "HUMAN", "subjectId" => "self-test-human" },
        "sessionEvidence" => {
          "provenance" => "CURRENT_CODEX_SESSION",
          "threadId" => "self-test-thread",
          "messages" => [approval_message],
          "checkedAt" => "2026-08-08T00:00:00Z"
        }
      }
      decision_ref = add_object(objects, "approved/human-decision.json", decision, "RFC8785_JSON")
      {
        "schemaVersion" => "codeclew-approval-bundle/0",
        "planStatus" => "PROPOSED_AWAITING_HUMAN_APPROVAL",
        "approvalSubject" => subject,
        "approvalSubjectDigest" => subject_digest,
        "humanDecisionRef" => role_ref("HUMAN_DECISION", decision_ref["ref"], decision_ref["sha256"]),
        "humanDecision" => decision,
        "createdAt" => "2026-08-08T00:00:00Z",
        "digestScope" => "RFC8785_CANONICAL_JSON"
      }
    end

    def approve_manifest_bundle!(case_data)
      approval = case_data.fetch("approvalBundle")
      subject = approval.fetch("approvalSubject")
      manifest_ref = subject.fetch("bootstrapArtifacts").find { |ref| ref["role"] == "BOOTSTRAP_MANIFESTS" }
      raise "self-test approval lacks BOOTSTRAP_MANIFESTS" unless manifest_ref

      stored_manifest = add_object(
        case_data.fetch("objectStore"),
        manifest_ref.fetch("ref"),
        JSON.pretty_generate(case_data.fetch("manifestBundle")) + "\n",
        "UTF8"
      )
      manifest_ref["sha256"] = stored_manifest.fetch("sha256")
      subject_digest = CanonicalJSON.sha256(subject)
      approval["approvalSubjectDigest"] = subject_digest
      decision = approval.fetch("humanDecision")
      decision["subjectDigest"] = subject_digest
      decision_ref = approval.fetch("humanDecisionRef")
      stored_decision = add_object(
        case_data.fetch("objectStore"),
        decision_ref.fetch("ref"),
        decision,
        "RFC8785_JSON"
      )
      decision_ref["sha256"] = stored_decision.fetch("sha256")
      case_data["packet"]["approvalBundleDigest"] = CanonicalJSON.sha256(approval)
      reseal!(case_data)
    end

    def resign_test_approval!(case_data)
      approval = case_data.fetch("approvalBundle")
      subject_digest = CanonicalJSON.sha256(approval.fetch("approvalSubject"))
      approval["approvalSubjectDigest"] = subject_digest
      decision = approval.fetch("humanDecision")
      decision["subjectDigest"] = subject_digest
      decision_ref = approval.fetch("humanDecisionRef")
      stored_decision = add_object(case_data.fetch("objectStore"), decision_ref.fetch("ref"), decision, "RFC8785_JSON")
      decision_ref["sha256"] = stored_decision["sha256"]
      case_data["packet"]["approvalBundleDigest"] = CanonicalJSON.sha256(approval)
      reseal!(case_data)
    end

    def build_base_case(node_id = "R02")
      manifest_bundle = JSON.parse(File.read(MANIFEST_PATH))
      template = manifest_bundle.fetch("manifests").find { |manifest| manifest["id"] == node_id }
      raise "missing #{node_id} manifest" unless template

      objects = []
      approval = build_approval(objects)
      plan_digest = approval.dig("approvalSubject", "plan", "sha256")
      bindings = {
        "PLAN_DIGEST" => plan_digest,
        "ATTEMPT" => "1",
        "EXHAUSTED_SOURCE" => ["R02", "R03"]
      }
      materialized = Materializer.new(bindings).materialize(template)
      output_paths = materialized["outputArtifacts"].reject { |path| path.end_with?("/packet.json") || path.end_with?("/summary.md") }
      artifacts = output_paths.map.with_index do |path, index|
        stored = add_object(objects, path, "artifact #{node_id} #{index}", "UTF8")
        { "path" => path, "sha256" => stored["sha256"], "sensitivity" => "PUBLIC" }
      end
      evidence_artifact = artifacts.first
      approved_sources = clone(approval.dig("approvalSubject", "sources"))
      parent_receipt = {
        "nodeId" => "R01",
        "attempt" => 1,
        "approvalBundleDigest" => CanonicalJSON.sha256(approval),
        "verdict" => "ACCEPT",
        "packetOutcome" => "SUCCESS",
        "packetBranchCode" => "NONE",
        "digestScope" => "RFC8785_CANONICAL_JSON_WITHOUT_RECEIPT_DIGEST",
        "receiptDigest" => "0" * 64
      }
      parent_receipt["receiptDigest"] = CanonicalJSON.sha256(Util.without_key(parent_receipt, "receiptDigest"))
      parent_ref = add_object(objects, "parents/R01/1/receipt.json", parent_receipt, "RECEIPT_WITHOUT_RECEIPT_DIGEST")
      telemetry = {
        "nativeTokenTelemetryAvailable" => true,
        "inputTokens" => 100,
        "cachedInputTokens" => 20,
        "outputTokens" => 10,
        "noncachedTokens" => 90,
        "toolCalls" => 4,
        "wallMilliseconds" => 1000,
        "visibleContextBytes" => 4096
      }
      packet = {
        "schemaVersion" => "codeclew-evidence-packet/0",
        "nodeId" => node_id,
        "attempt" => 1,
        "startedAt" => "2026-08-07T23:59:58.500Z",
        "producerCompletedAt" => "2026-08-07T23:59:59.500Z",
        "outcome" => "SUCCESS",
        "branchCode" => node_id == "GK" ? "INCONCLUSIVE_FOUNDATION" : "NONE",
        "limitations" => [],
        "hypothesisIds" => clone(materialized["hypothesisIds"]),
        "producer" => {
          "agentId" => "producer-#{node_id.downcase}",
          "sessionId" => "producer-session-#{node_id.downcase}",
          "modelVersion" => "self-test-model",
          "toolVersions" => { "ruby" => RUBY_VERSION }
        },
        "approvalBundleDigest" => CanonicalJSON.sha256(approval),
        "runManifestDigest" => CanonicalJSON.sha256(materialized),
        "sourceDigests" => clone(approved_sources),
        "parentReceiptDigests" => %w[R01 GK].include?(node_id) ? [] : [parent_ref],
        "artifactSetDigest" => CanonicalJSON.sha256(output_paths.sort),
        "metricEligibility" => { "nativeTokens" => "AVAILABLE" },
        "claims" => [
          {
            "claimId" => "#{node_id}-SELF-TEST-C1",
            "statement" => "The planning bootstrap controller accepted the synthetic positive case.",
            "label" => "TEST",
            "domains" => [node_id == "GK" ? "FOUNDATION" : "CONTRACT"],
            "evidenceRefs" => [digest_ref(evidence_artifact["path"], evidence_artifact["sha256"])],
            "falsifiers" => ["Any required controller invariant fails."],
            "coverageBoundary" => "Synthetic planning-side self-test only"
          }
        ],
        "evidenceDelta" => {
          "kind" => "PREREQUISITE_CREATED",
          "statement" => "A deterministic planning-side bootstrap verification prerequisite was exercised.",
          "domains" => [node_id == "GK" ? "FOUNDATION" : "CONTRACT"],
          "artifactRefs" => [digest_ref(evidence_artifact["path"], evidence_artifact["sha256"])]
        },
        "telemetry" => telemetry,
        "artifacts" => artifacts,
        "humanReadableConclusion" => "The synthetic planning-side bootstrap controller case was accepted.",
        "proposedNextEdges" => case node_id
                               when "R01" then ["R01->R02", "R01->R03"]
                               when "GK" then ["GK->GF0"]
                               else ["#{node_id}->K01"]
                               end
      }
      verifier = {
        "nativeTokenTelemetryAvailable" => true,
        "inputTokens" => 50,
        "cachedInputTokens" => 0,
        "outputTokens" => 5,
        "noncachedTokens" => 55,
        "toolCalls" => 2,
        "wallMilliseconds" => 500,
        "visibleContextBytes" => 2048
      }
      totals = {
        "nativeTokenTelemetryAvailable" => true,
        "inputTokens" => 150,
        "cachedInputTokens" => 20,
        "outputTokens" => 15,
        "noncachedTokens" => 145,
        "toolCalls" => 6,
        "teamWallMilliseconds" => 1500,
        "maxVisibleContextBytes" => 4096
      }
      checks = materialized["requiredCheckIds"].map do |check_id|
        {
          "checkId" => check_id,
          "result" => "PASS",
          "evidenceRef" => digest_ref(evidence_artifact["path"], evidence_artifact["sha256"]),
          "explanation" => "Synthetic self-test evidence resolved for #{check_id}."
        }
      end
      receipt = {
        "schemaVersion" => "codeclew-verification-receipt/0",
        "packetDigest" => CanonicalJSON.sha256(packet),
        "approvalBundleDigest" => packet["approvalBundleDigest"],
        "runManifestDigest" => packet["runManifestDigest"],
        "nodeId" => node_id,
        "attempt" => 1,
        "verifier" => {
          "agentId" => "verifier-#{node_id.downcase}",
          "sessionId" => "verifier-session-#{node_id.downcase}",
          "modelVersion" => "self-test-model",
          "toolVersions" => { "ruby" => RUBY_VERSION }
        },
        "producerSessionId" => packet.dig("producer", "sessionId"),
        "independenceAttestation" => true,
        "checks" => checks,
        "verdict" => "ACCEPT",
        "packetOutcome" => packet["outcome"],
        "packetBranchCode" => packet["branchCode"],
        "costAccounting" => {
          "budgetRefDigest" => CanonicalJSON.sha256(materialized["budgets"]),
          "producerTelemetryDigest" => CanonicalJSON.sha256(telemetry),
          "verifierTelemetry" => verifier,
          "priorAttemptReceipts" => [],
          "teamTotals" => totals,
          "budgetStatus" => "WITHIN",
          "exceededMetrics" => []
        },
        "limitations" => [],
        "verifiedAt" => "2026-08-08T00:00:00Z",
        "digestScope" => "RFC8785_CANONICAL_JSON_WITHOUT_RECEIPT_DIGEST",
        "receiptDigest" => "0" * 64
      }
      receipt["receiptDigest"] = CanonicalJSON.sha256(Util.without_key(receipt, "receiptDigest"))

      runtime = {
        "dotDigest" => approval.dig("approvalSubject", "authoritativeDot", "sha256"),
        "allDigestsCurrent" => true,
        "currentSourceDigests" => clone(approved_sources),
        "expectedParentReceiptDigests" => packet["parentReceiptDigests"],
        "authorizedEvidenceRefs" => [],
        "preexistingSuccessArtifacts" => [],
        "attemptHistory" => [],
        "retryState" => "NOT_APPLICABLE",
        "edgeRegistry" => DotEdgeRegistry.for_source(File.read(DOT_PATH), node_id)
      }
      case_data = {
        "mode" => "TEST_ONLY",
        "approvalBundle" => approval,
        "manifestBundle" => manifest_bundle,
        "bindings" => bindings,
        "packet" => packet,
        "receipt" => receipt,
        "runtimeState" => runtime,
        "objectStore" => objects
      }
      sync_self_artifacts!(case_data)
      attest_runtime!(case_data)
      case_data
    end

    def materialized_manifest(case_data)
      node_id = case_data.dig("packet", "nodeId")
      template = case_data.fetch("manifestBundle").fetch("manifests").find { |manifest| manifest["id"] == node_id }
      Materializer.new(case_data.fetch("bindings")).materialize(template)
    end

    def add_artifact!(case_data, path, content)
      stored = add_object(case_data["objectStore"], path, content, "UTF8")
      { "path" => path, "sha256" => stored["sha256"], "sensitivity" => "PUBLIC" }
    end

    def sync_self_artifacts!(case_data)
      manifest = materialized_manifest(case_data)
      policy = ArtifactPolicy.new(manifest, case_data.dig("packet", "outcome"), case_data.dig("packet", "branchCode"))
      self_paths = policy.records.select { |record| record["selfReferential"] }.map { |record| record["path"] }
      packet_path = self_paths.find { |path| path.end_with?("/packet.json") }
      summary_path = self_paths.find { |path| path.end_with?("/summary.md") }
      raise "self-test manifest lacks packet/summary self artifacts" unless packet_path && summary_path
      packet_ref = add_object(case_data.fetch("objectStore"), packet_path, case_data.fetch("packet"), "RFC8785_JSON")
      summary = "#{case_data.dig('packet', 'nodeId')} attempt #{case_data.dig('packet', 'attempt')} receipt #{case_data.dig('receipt', 'receiptDigest')}\n"
      summary_ref = add_object(case_data.fetch("objectStore"), summary_path, summary, "UTF8")
      case_data["selfArtifacts"] = { "packet" => packet_ref, "summary" => summary_ref }
    end

    def point_evidence_to!(case_data, artifact)
      ref = digest_ref(artifact["path"], artifact["sha256"])
      case_data["packet"]["claims"].each { |claim| claim["evidenceRefs"] = [clone(ref)] }
      case_data["packet"]["evidenceDelta"]["artifactRefs"] = [clone(ref)]
      case_data["receipt"]["checks"].each { |check| check["evidenceRef"] = clone(ref) }
    end

    def sync_success_artifacts!(case_data)
      manifest = materialized_manifest(case_data)
      paths = manifest["outputArtifacts"].reject { |path| path.end_with?("/packet.json") || path.end_with?("/summary.md") }
      artifacts = paths.map.with_index { |path, index| add_artifact!(case_data, path, "synced success artifact #{index}") }
      case_data["packet"]["artifacts"] = artifacts
      case_data["packet"]["artifactSetDigest"] = CanonicalJSON.sha256(paths.sort)
      point_evidence_to!(case_data, artifacts.first)
      case_data["packet"]["runManifestDigest"] = CanonicalJSON.sha256(manifest)
      case_data["receipt"]["runManifestDigest"] = case_data["packet"]["runManifestDigest"]
    end

    def sync_generic_artifacts!(case_data, include_success_artifact: false)
      manifest = materialized_manifest(case_data)
      paths = manifest["genericOutcomeArtifacts"].reject { |path| path.end_with?("/packet.json") || path.end_with?("/summary.md") }
      artifacts = paths.map.with_index { |path, index| add_artifact!(case_data, path, "generic diagnostic #{index}") }
      if include_success_artifact
        success_path = manifest["outputArtifacts"].find { |path| !path.end_with?("/packet.json") && !path.end_with?("/summary.md") }
        success_artifact = add_artifact!(case_data, success_path, "preexisting immutable success artifact")
        artifacts << success_artifact
        case_data["runtimeState"]["preexistingSuccessArtifacts"] = [digest_ref(success_artifact["path"], success_artifact["sha256"])]
      end
      case_data["packet"]["artifacts"] = artifacts
      case_data["packet"]["artifactSetDigest"] = CanonicalJSON.sha256(artifacts.map { |artifact| artifact["path"] }.sort)
      point_evidence_to!(case_data, artifacts.first)
    end

    def set_unavailable_telemetry!(case_data)
      packet = case_data["packet"]
      receipt = case_data["receipt"]
      packet["telemetry"] = {
        "nativeTokenTelemetryAvailable" => false,
        "inputTokens" => nil,
        "cachedInputTokens" => nil,
        "outputTokens" => nil,
        "noncachedTokens" => nil,
        "toolCalls" => 4,
        "wallMilliseconds" => 1000,
        "visibleContextBytes" => 4096
      }
      packet["metricEligibility"] = { "nativeTokens" => "UNAVAILABLE" }
      packet["branchCode"] = "TOKEN_TELEMETRY_UNAVAILABLE"
      receipt["costAccounting"]["verifierTelemetry"] = {
        "nativeTokenTelemetryAvailable" => false,
        "inputTokens" => nil,
        "cachedInputTokens" => nil,
        "outputTokens" => nil,
        "noncachedTokens" => nil,
        "toolCalls" => 2,
        "wallMilliseconds" => 500,
        "visibleContextBytes" => 2048
      }
      receipt["costAccounting"]["teamTotals"] = {
        "nativeTokenTelemetryAvailable" => false,
        "inputTokens" => nil,
        "cachedInputTokens" => nil,
        "outputTokens" => nil,
        "noncachedTokens" => nil,
        "toolCalls" => 6,
        "teamWallMilliseconds" => 1500,
        "maxVisibleContextBytes" => 4096
      }
      receipt["costAccounting"]["budgetStatus"] = "TOKEN_TELEMETRY_UNAVAILABLE"
      receipt["costAccounting"]["exceededMetrics"] = []
      reseal!(case_data)
    end

    def set_verifier_token_unavailable!(case_data)
      receipt = case_data.fetch("receipt")
      receipt.fetch("costAccounting")["verifierTelemetry"] = {
        "nativeTokenTelemetryAvailable" => false,
        "inputTokens" => nil,
        "cachedInputTokens" => nil,
        "outputTokens" => nil,
        "noncachedTokens" => nil,
        "toolCalls" => 2,
        "wallMilliseconds" => 500,
        "visibleContextBytes" => 2048
      }
      receipt.fetch("costAccounting")["teamTotals"] = {
        "nativeTokenTelemetryAvailable" => false,
        "inputTokens" => nil,
        "cachedInputTokens" => nil,
        "outputTokens" => nil,
        "noncachedTokens" => nil,
        "toolCalls" => 6,
        "teamWallMilliseconds" => 1500,
        "maxVisibleContextBytes" => 4096
      }
      receipt.fetch("costAccounting")["budgetStatus"] = "TOKEN_TELEMETRY_UNAVAILABLE"
      receipt.fetch("costAccounting")["exceededMetrics"] = []
      reseal!(case_data)
    end

    def set_over_budget!(case_data)
      budget = materialized_manifest(case_data)["budgets"]
      producer_noncached = budget["noncachedTokenCeiling"] + 1
      case_data["packet"]["telemetry"].merge!(
        "inputTokens" => producer_noncached,
        "cachedInputTokens" => 0,
        "outputTokens" => 0,
        "noncachedTokens" => producer_noncached
      )
      verifier = case_data.dig("receipt", "costAccounting", "verifierTelemetry")
      totals = case_data.dig("receipt", "costAccounting", "teamTotals")
      totals.merge!(
        "inputTokens" => producer_noncached + verifier["inputTokens"],
        "cachedInputTokens" => verifier["cachedInputTokens"],
        "outputTokens" => verifier["outputTokens"],
        "noncachedTokens" => producer_noncached + verifier["noncachedTokens"]
      )
      case_data["receipt"]["costAccounting"]["budgetStatus"] = "EXCEEDED"
      case_data["receipt"]["costAccounting"]["exceededMetrics"] = ["NONCACHED_TOKENS"]
      reseal!(case_data)
    end

    def ajv_validate(schema, value, label)
      Tempfile.create(["codeclew-self-test-#{label}", ".json"]) do |file|
        file.write(JSON.generate(value))
        file.flush
        stdout, stderr, status = Open3.capture3("npx", "--yes", "ajv-cli@5", "validate", "--spec=draft2020", "-s", schema, "-d", file.path)
        raise "AJV rejected #{label}: #{stdout} #{stderr}" unless status.success?
      end
    end

    def ajv_expect_reject(schema, value, label)
      Tempfile.create(["codeclew-self-test-invalid-#{label}", ".json"]) do |file|
        file.write(JSON.generate(value))
        file.flush
        _stdout, _stderr, status = Open3.capture3("npx", "--yes", "ajv-cli@5", "validate", "--spec=draft2020", "-s", schema, "-d", file.path)
        raise "AJV unexpectedly accepted invalid #{label}" if status.success?
      end
    end

    def test_ajv
      case_data = build_base_case
      ajv_validate(APPROVAL_SCHEMA_PATH, case_data["approvalBundle"], "approval")
      ajv_validate(NODE_SCHEMA_PATH, case_data["manifestBundle"], "manifests")
      ajv_validate(PACKET_SCHEMA_PATH, case_data["packet"], "packet")
      ajv_validate(RECEIPT_SCHEMA_PATH, case_data["receipt"], "receipt")
    end

    def test_ajv_rejections
      case_data = build_base_case

      approval = clone(case_data["approvalBundle"])
      approval.dig("humanDecision", "sessionEvidence", "messages", 0).delete("messageId")
      ajv_expect_reject(APPROVAL_SCHEMA_PATH, approval, "approval-missing-observed-message-id")

      manifests = clone(case_data["manifestBundle"])
      manifests["manifests"] << clone(manifests["manifests"].first)
      ajv_expect_reject(NODE_SCHEMA_PATH, manifests, "manifest-duplicate-object")

      packet = clone(case_data["packet"])
      packet.delete("artifactSetDigest")
      ajv_expect_reject(PACKET_SCHEMA_PATH, packet, "packet-missing-artifact-set-digest")

      receipt = clone(case_data["receipt"])
      receipt.delete("attempt")
      ajv_expect_reject(RECEIPT_SCHEMA_PATH, receipt, "receipt-missing-attempt")
    end

    def test_materializer
      value = {
        "nested" => [
          "objects/${PLAN_DIGEST}/${SOURCE}/packet.json",
          { "attempt" => "${ATTEMPT}" }
        ]
      }
      result = Materializer.new(
        "PLAN_DIGEST" => "a" * 64,
        "SOURCE" => %w[R02 R03],
        "ATTEMPT" => "${ATTEMPT_NUMBER}",
        "ATTEMPT_NUMBER" => 2
      ).materialize(value)
      expected_paths = ["objects/#{'a' * 64}/R02/packet.json", "objects/#{'a' * 64}/R03/packet.json"]
      Util.assert(result["nested"].first(2) == expected_paths && result.dig("nested", 2, "attempt") == 2,
                  "SELF_TEST_MATERIALIZER", "recursive materializer did not expand as expected", result)
      expect_reject("PLACEHOLDER_CYCLE") do
        Materializer.new("A" => "${B}", "B" => "${A}").materialize("${A}")
      end
      expect_reject("UNKNOWN_PLACEHOLDER") do
        Materializer.new("A" => "present").materialize("${MISSING}")
      end
    end

    def test_fixture_spec
      spec = JSON.parse(File.read(FIXTURE_SPEC_PATH))
      Util.assert(spec["schemaVersion"] == "codeclew-bootstrap-controller-self-test/0",
                  "SELF_TEST_FIXTURE_SPEC", "fixture spec schemaVersion is wrong")
      Util.assert(spec["generation"] == "DYNAMIC_FROM_CURRENT_BOOTSTRAP_MANIFESTS",
                  "SELF_TEST_FIXTURE_SPEC", "fixture spec must prohibit copied frozen vectors")

      positive = (spec["positiveChains"] || []).each_with_object({}) do |entry, index|
        Util.assert(entry.is_a?(Hash) && entry["id"].is_a?(String), "SELF_TEST_FIXTURE_SPEC", "positive fixture entry is invalid", entry)
        Util.assert(!index.key?(entry["id"]), "SELF_TEST_FIXTURE_SPEC", "positive fixture IDs must be unique", entry["id"])
        index[entry["id"]] = entry
      end
      adversarial = (spec["adversarialCases"] || []).each_with_object({}) do |entry, index|
        Util.assert(entry.is_a?(Hash) && entry["id"].is_a?(String), "SELF_TEST_FIXTURE_SPEC", "adversarial fixture entry is invalid", entry)
        Util.assert(!index.key?(entry["id"]), "SELF_TEST_FIXTURE_SPEC", "adversarial fixture IDs must be unique", entry["id"])
        index[entry["id"]] = entry
      end

      expected_positive = {
        "R02_SUCCESS_AVAILABLE" => { "expectedEffectiveEdges" => ["R02->K01"] },
        "R02_GENERIC_EXHAUSTED" => { "expectedEffectiveEdges" => ["R02->GK"] },
        "R02_SUCCESS_TOKEN_UNAVAILABLE" => { "expectedEffectiveEdges" => ["R02->K01"] },
        "R02_MIXED_TEAM_TOKEN_UNAVAILABLE" => {
          "expectedBudgetStatus" => "TOKEN_TELEMETRY_UNAVAILABLE",
          "expectedEffectiveEdges" => ["R02->K01"]
        },
        "R02_OVER_BUDGET_PROJECTION" => {
          "expectedEffectiveOutcome" => "NO_PROGRESS",
          "expectedEffectiveBranchCode" => "BUDGET_EXCEEDED",
          "expectedEffectiveEdges" => ["R02->GK"]
        },
        "R02_ATTEMPT_2_CUMULATIVE" => { "expectedControllerVerdict" => "CONTROL_ACCEPT" },
        "R01_REORDERED_APPROVED_SOURCES" => {
          "expectedControllerVerdict" => "CONTROL_ACCEPT",
          "expectedEffectiveEdges" => ["R01->R02", "R01->R03"]
        },
        "GK_EXACT_R02_R03_QUIESCENT" => { "expectedEffectiveEdges" => ["GK->GF0"] },
        "CODEX_READ_ONLY_APPROVAL_V1" => { "expectedControllerVerdict" => "CONTROL_ACCEPT" }
      }
      expected_adversarial = {
        "TEST_ONLY_NORMAL_RUNTIME" => { "expectedRejectCode" => "TEST_ONLY_FORBIDDEN" },
        "APPROVAL_SUBJECT_TAMPER" => { "expectedRejectCode" => "APPROVAL_SUBJECT_DIGEST_MISMATCH" },
        "SESSION_APPROVAL_NON_USER" => { "expectedRejectCode" => "SESSION_APPROVAL_IDENTITY_MISMATCH" },
        "UNAPPROVED_MANIFEST_SUBSTITUTION" => { "expectedRejectCode" => "APPROVED_MANIFEST_BUNDLE_MISMATCH" },
        "EXECUTING_CONTROLLER_DIGEST_MISMATCH" => { "expectedRejectCode" => "EXECUTING_CONTROLLER_DIGEST_MISMATCH" },
        "COORDINATED_APPROVED_SOURCE_SUBSTITUTION" => { "expectedRejectCode" => "RUNTIME_APPROVED_SOURCE_SET_MISMATCH" },
        "MISSING_APPROVED_SOURCE" => { "expectedRejectCode" => "INVALID_PACKET_SOURCE_SET" },
        "EXTRA_APPROVED_SOURCE" => { "expectedRejectCode" => "INVALID_PACKET_SOURCE_SET" },
        "DUPLICATE_APPROVED_SOURCE_ROLE" => { "expectedRejectCode" => "INVALID_PACKET_SOURCE_SET" },
        "APPROVED_SOURCE_ROLE_MISMATCH" => { "expectedRejectCode" => "INVALID_PACKET_SOURCE_SET" },
        "PRIOR_APPROVED_SOURCE_SUBSTITUTION" => { "expectedRejectCode" => "PRIOR_SOURCE_APPROVAL_SET_MISMATCH" },
        "DUPLICATE_MANIFEST_ID" => { "expectedRejectCode" => "MANIFEST_ID_CARDINALITY" },
        "ACCEPT_WITH_MANDATORY_FAIL" => { "expectedRejectCode" => "MANDATORY_CHECK_FAILED" },
        "MISSING_REQUIRED_CHECK_ID" => { "expectedRejectCode" => "MANDATORY_CHECK_SET_MISMATCH" },
        "WRONG_REQUIRED_CHECK_ID" => { "expectedRejectCode" => "MANDATORY_CHECK_SET_MISMATCH" },
        "DUPLICATE_REQUIRED_CHECK_ID" => { "expectedRejectCode" => "MANDATORY_CHECK_SET_MISMATCH" },
        "WRONG_HYPOTHESIS_EXACT_SET" => { "expectedRejectCode" => "HYPOTHESIS_SET_MISMATCH" },
        "SAME_PRODUCER_VERIFIER_SESSION" => { "expectedRejectCode" => "SESSION_INDEPENDENCE_VIOLATION" },
        "INVERTED_ATTEMPT_TIMESTAMPS" => { "expectedRejectCode" => "NON_MONOTONIC_EVENT_CLOCK" },
        "SUCCESS_MISSING_ARTIFACT" => { "expectedRejectCode" => "SUCCESS_ARTIFACT_SET_MISMATCH" },
        "GENERIC_MISSING_DIAGNOSTIC" => { "expectedRejectCode" => "MISSING_GENERIC_DIAGNOSTIC" },
        "GENERIC_UNDECLARED_ARTIFACT" => { "expectedRejectCode" => "UNDECLARED_PACKET_ARTIFACT" },
        "MISSING_SUMMARY_SELF_ARTIFACT" => { "expectedRejectCode" => "SELF_ARTIFACT_SET_MISMATCH" },
        "MISMATCHED_PACKET_SELF_ARTIFACT" => { "expectedRejectCode" => "PACKET_SELF_OBJECT_MISMATCH" },
        "EMPTY_SUMMARY_SELF_ARTIFACT" => { "expectedRejectCode" => "SUMMARY_SELF_OBJECT_EMPTY" },
        "TOKEN_DOMAIN_WITHOUT_NATIVE_TELEMETRY" => { "expectedRejectCode" => "TOKEN_DOMAIN_FORBIDDEN" },
        "AVAILABLE_TOKEN_ELIGIBILITY_MISMATCH" => { "expectedRejectCode" => "METRIC_ELIGIBILITY_MISMATCH" },
        "DANGLING_EVIDENCE_REF" => { "expectedRejectCode" => "DANGLING_DIGEST_REF" },
        "HASH_MISMATCH_EVIDENCE_REF" => { "expectedRejectCode" => "EVIDENCE_REF_DIGEST_MISMATCH" },
        "NONCACHED_FORMULA_UNDERCOUNT" => { "expectedRejectCode" => "TOKEN_TELEMETRY_FORMULA_MISMATCH" },
        "ATTEMPT_2_MISSING_ANCESTRY" => { "expectedRejectCode" => "RETRY_ANCESTRY_MISMATCH" },
        "PRIOR_RECEIPT_DIGEST_MISMATCH" => { "expectedRejectCode" => "PRIOR_RECEIPT_DIGEST_MISMATCH" },
        "PRIOR_SUCCESS_NOT_RETRYABLE" => { "expectedRejectCode" => "PRIOR_ATTEMPT_NOT_RETRYABLE" },
        "PRIOR_EXCEEDED_NOT_RETRYABLE" => { "expectedRejectCode" => "PRIOR_ATTEMPT_NOT_RETRYABLE" },
        "PRIOR_BUDGET_BRANCH_NOT_RETRYABLE" => { "expectedRejectCode" => "PRIOR_ATTEMPT_NOT_RETRYABLE" },
        "PRIOR_ARTIFACT_SET_TAMPER" => { "expectedRejectCode" => "MISSING_GENERIC_DIAGNOSTIC" },
        "PRIOR_SELF_REF_MISMATCH" => { "expectedRejectCode" => "PRIOR_SELF_ARTIFACT_REF_MISMATCH" },
        "FALSE_PROPOSED_EDGE" => {
          "expectedControllerVerdict" => "CONTROL_ACCEPT",
          "expectedEffectiveEdges" => ["R02->K01"],
          "expectedProducerEdgeHintMatches" => false
        },
        "UNAPPROVED_HOST_EDGE_REGISTRY_ROW" => { "expectedRejectCode" => "EDGE_REGISTRY_DOT_MISMATCH" },
        "GK_MISSING_PARENT" => { "expectedRejectCode" => "PARENT_RECEIPT_SET_MISMATCH" },
        "GK_PARENT_NOT_EXHAUSTED" => { "expectedRejectCode" => "GK_PARENT_NOT_EXHAUSTED" },
        "GK_NON_QUIESCENT" => { "expectedRejectCode" => "GK_WAVE_NOT_QUIESCENT" },
        "GK_WRONG_BRANCH" => { "expectedRejectCode" => "GK_BOOTSTRAP_OUTPUT_INVALID" },
        "SESSION_APPROVAL_WRONG_THREAD" => { "expectedRejectCode" => "SESSION_APPROVAL_IDENTITY_MISMATCH" },
        "SESSION_APPROVAL_DIGEST_MISMATCH" => { "expectedRejectCode" => "SESSION_APPROVAL_DIGEST_MISMATCH" },
        "RUNTIME_STATE_DIGEST_MISMATCH" => { "expectedRejectCode" => "RUNTIME_STATE_DIGEST_MISMATCH" },
        "RUNTIME_OBSERVER_STATE_MISMATCH" => { "expectedRejectCode" => "INVALID_RUNTIME_OBSERVATION" },
        "RUNTIME_OBSERVER_DUPLICATE_SESSION" => { "expectedRejectCode" => "RUNTIME_OBSERVER_INDEPENDENCE_VIOLATION" },
        "AUTHORITY_PRODUCER_SESSION_COLLISION" => { "expectedRejectCode" => "AUTHORITY_SESSION_INDEPENDENCE_VIOLATION" },
        "PLACEHOLDER_CYCLE" => { "expectedRejectCode" => "PLACEHOLDER_CYCLE" },
        "UNKNOWN_PLACEHOLDER" => { "expectedRejectCode" => "UNKNOWN_PLACEHOLDER" }
      }

      assert_fixture_entries!(positive, expected_positive, "positiveChains")
      assert_fixture_entries!(adversarial, expected_adversarial, "adversarialCases")
      expected_schema_invalid = {
        "APPROVAL_MISSING_OBSERVED_MESSAGE_ID" => { "schema" => "APPROVAL_BUNDLE_SCHEMA" },
        "MANIFEST_DUPLICATE_OBJECT" => { "schema" => "NODE_CONTRACT_SCHEMA" },
        "PACKET_MISSING_ARTIFACT_SET_DIGEST" => { "schema" => "EVIDENCE_PACKET_SCHEMA" },
        "RECEIPT_MISSING_ATTEMPT" => { "schema" => "VERIFICATION_RECEIPT_SCHEMA" }
      }
      schema_invalid = (spec["schemaInvalidCases"] || []).each_with_object({}) do |entry, index|
        Util.assert(entry.is_a?(Hash) && entry["id"].is_a?(String), "SELF_TEST_FIXTURE_SPEC", "schema-invalid fixture entry is invalid", entry)
        Util.assert(!index.key?(entry["id"]), "SELF_TEST_FIXTURE_SPEC", "schema-invalid fixture IDs must be unique", entry["id"])
        index[entry["id"]] = entry
      end
      assert_fixture_entries!(schema_invalid, expected_schema_invalid, "schemaInvalidCases")
      invariants = spec["invariants"]
      Util.assert(invariants.is_a?(Array) && invariants.length == 8 && invariants.all? { |item| item.is_a?(String) && !item.empty? },
                  "SELF_TEST_FIXTURE_SPEC", "fixture spec must contain the eight non-empty controller invariants")
    end

    def assert_fixture_entries!(actual, expected, collection)
      Util.assert(actual.keys.sort == expected.keys.sort, "SELF_TEST_FIXTURE_SPEC", "#{collection} IDs differ from executable self-tests",
                  { "expected" => expected.keys.sort, "actual" => actual.keys.sort })
      expected.each do |id, fields|
        entry = actual.fetch(id)
        fields.each do |key, value|
          Util.assert(entry[key] == value, "SELF_TEST_FIXTURE_SPEC", "#{collection}.#{id}.#{key} is wrong",
                      { "expected" => value, "actual" => entry[key] })
        end
      end
    end

    def test_baseline
      r01 = build_base_case("R01")
      ajv_validate(PACKET_SCHEMA_PATH, r01["packet"], "chain-r01-packet")
      ajv_validate(RECEIPT_SCHEMA_PATH, r01["receipt"], "chain-r01-receipt")
      r01_result = controller.verify(r01)
      Util.assert(r01_result["effectiveEligibleNextEdges"] == ["R01->R02", "R01->R03"],
                  "SELF_TEST_BASELINE", "R01 did not expose the exact approved bootstrap continuations", r01_result)

      case_data = build_base_case
      add_object(case_data["objectStore"], "chain/R01/1/packet.json", r01["packet"], "RFC8785_JSON")
      parent_ref = add_object(case_data["objectStore"], "chain/R01/1/receipt.json", r01["receipt"], "RECEIPT_WITHOUT_RECEIPT_DIGEST")
      case_data["packet"]["parentReceiptDigests"] = [parent_ref]
      case_data["runtimeState"]["expectedParentReceiptDigests"] = [clone(parent_ref)]
      reseal!(case_data)
      attest_runtime!(case_data)
      ajv_validate(PACKET_SCHEMA_PATH, case_data["packet"], "chain-r02-packet")
      ajv_validate(RECEIPT_SCHEMA_PATH, case_data["receipt"], "chain-r02-receipt")
      result = controller.verify(case_data)
      Util.assert(result["effectiveEligibleNextEdges"] == ["R02->K01"], "SELF_TEST_BASELINE", "baseline effective edge mismatch", result)
      Util.assert(result["producerEdgeHintMatches"] == true, "SELF_TEST_BASELINE", "baseline producer edge hint should match")
    end

    def test_test_only_boundary
      expect_reject("TEST_ONLY_FORBIDDEN") do
        Controller.new(self_test: false).verify(build_base_case)
      end
    end

    def test_approval_tamper
      case_data = build_base_case
      case_data["approvalBundle"]["approvalSubject"]["planStatus"] = "TAMPERED"
      expect_reject("APPROVAL_SUBJECT_DIGEST_MISMATCH") { controller.verify(case_data) }
    end

    def test_session_approval_binding
      case_data = build_base_case
      approval = case_data["approvalBundle"]
      approval.dig("humanDecision", "sessionEvidence", "messages", 0)["authorRole"] = "ASSISTANT"
      decision_ref = approval["humanDecisionRef"]
      updated = add_object(case_data["objectStore"], decision_ref["ref"], approval["humanDecision"], "RFC8785_JSON")
      decision_ref["sha256"] = updated["sha256"]
      case_data["packet"]["approvalBundleDigest"] = CanonicalJSON.sha256(approval)
      reseal!(case_data)
      expect_reject("SESSION_APPROVAL_IDENTITY_MISMATCH") { controller.verify(case_data) }
    end

    def test_manifest_substitution
      case_data = build_base_case
      manifest = case_data.fetch("manifestBundle").fetch("manifests").find { |item| item["id"] == "R02" }
      manifest.fetch("budgets")["toolCallCeiling"] += 1
      expect_reject("APPROVED_MANIFEST_BUNDLE_MISMATCH") { controller.verify(case_data) }
    end

    def test_controller_digest_binding
      case_data = build_base_case
      ref = case_data.dig("approvalBundle", "approvalSubject", "bootstrapArtifacts").find do |item|
        item["role"] == "BOOTSTRAP_CONTROLLER"
      end
      stored = add_object(case_data["objectStore"], ref["ref"], "not the executing controller\n", "UTF8")
      ref["sha256"] = stored["sha256"]
      resign_test_approval!(case_data)
      expect_reject("EXECUTING_CONTROLLER_DIGEST_MISMATCH") { controller.verify(case_data) }
    end

    def test_approved_source_closure
      reordered = build_base_case("R01")
      reordered["packet"]["sourceDigests"] = reordered["packet"]["sourceDigests"].reverse
      reordered["runtimeState"]["currentSourceDigests"] = reordered["runtimeState"]["currentSourceDigests"].rotate(2)
      reseal!(reordered)
      attest_runtime!(reordered)
      reordered_result = controller.verify(reordered)
      Util.assert(reordered_result["controllerVerdict"] == "CONTROL_ACCEPT" &&
                  reordered_result["effectiveEligibleNextEdges"] == ["R01->R02", "R01->R03"],
                  "SELF_TEST_SOURCE_SET_ORDER", "approved source equality must be order-insensitive", reordered_result)

      coordinated = build_base_case("R01")
      stored = add_object(coordinated["objectStore"], "attack/substituted-S0", "substituted source S0", "UTF8")
      replacement = role_ref("S0", stored["ref"], stored["sha256"])
      coordinated["packet"]["sourceDigests"] = clone(coordinated["packet"]["sourceDigests"]).map do |ref|
        ref["role"] == "S0" ? clone(replacement) : ref
      end
      coordinated["runtimeState"]["currentSourceDigests"] = clone(coordinated["packet"]["sourceDigests"])
      reseal!(coordinated)
      attest_runtime!(coordinated)
      expect_reject("RUNTIME_APPROVED_SOURCE_SET_MISMATCH") { controller.verify(coordinated) }

      missing = build_base_case("R01")
      missing["packet"]["sourceDigests"].pop
      reseal!(missing)
      expect_reject("INVALID_PACKET_SOURCE_SET") { controller.verify(missing) }

      extra = build_base_case("R01")
      extra["packet"]["sourceDigests"] << clone(extra["packet"]["sourceDigests"].first)
      reseal!(extra)
      expect_reject("INVALID_PACKET_SOURCE_SET") { controller.verify(extra) }

      duplicate = build_base_case("R01")
      duplicate["packet"]["sourceDigests"][-1] = clone(duplicate["packet"]["sourceDigests"].first)
      reseal!(duplicate)
      expect_reject("INVALID_PACKET_SOURCE_SET") { controller.verify(duplicate) }

      role_mismatch = build_base_case("R01")
      role_mismatch["packet"]["sourceDigests"].first["role"] = "PLAN"
      reseal!(role_mismatch)
      expect_reject("INVALID_PACKET_SOURCE_SET") { controller.verify(role_mismatch) }

      prior = promote_to_attempt_two!(build_base_case)
      prior_stored = add_object(prior["objectStore"], "attack/prior-substituted-S0", "prior substituted source S0", "UTF8")
      prior_replacement = role_ref("S0", prior_stored["ref"], prior_stored["sha256"])
      replace_prior_pair!(prior) do |packet, _receipt|
        packet["sourceDigests"] = clone(packet["sourceDigests"]).map do |ref|
          ref["role"] == "S0" ? clone(prior_replacement) : ref
        end
      end
      expect_reject("PRIOR_SOURCE_APPROVAL_SET_MISMATCH") { controller.verify(prior) }
    end

    def test_duplicate_manifest_id
      case_data = build_base_case
      manifest = case_data.fetch("manifestBundle").fetch("manifests").find { |item| item["id"] == "R02" }
      case_data.fetch("manifestBundle").fetch("manifests") << clone(manifest)
      approve_manifest_bundle!(case_data)
      expect_reject("MANIFEST_ID_CARDINALITY") { controller.verify(case_data) }
    end

    def test_accept_with_failed_check
      case_data = build_base_case
      case_data["receipt"]["checks"].first["result"] = "FAIL"
      reseal!(case_data)
      expect_reject("MANDATORY_CHECK_FAILED") { controller.verify(case_data) }
    end

    def test_required_check_id_mutations
      missing = build_base_case
      missing["receipt"]["checks"].pop
      reseal!(missing)
      expect_reject("MANDATORY_CHECK_SET_MISMATCH") { controller.verify(missing) }

      wrong = build_base_case
      wrong["receipt"]["checks"].first["checkId"] = "WRONG_CHECK_ID"
      reseal!(wrong)
      expect_reject("MANDATORY_CHECK_SET_MISMATCH") { controller.verify(wrong) }

      duplicate = build_base_case
      duplicate["receipt"]["checks"][1]["checkId"] = duplicate["receipt"]["checks"][0]["checkId"]
      reseal!(duplicate)
      expect_reject("MANDATORY_CHECK_SET_MISMATCH") { controller.verify(duplicate) }
    end

    def test_hypothesis_set
      case_data = build_base_case
      case_data["packet"]["hypothesisIds"].pop
      reseal!(case_data)
      expect_reject("HYPOTHESIS_SET_MISMATCH") { controller.verify(case_data) }
    end

    def test_session_independence
      case_data = build_base_case
      case_data["receipt"]["verifier"]["sessionId"] = case_data.dig("packet", "producer", "sessionId")
      reseal!(case_data)
      expect_reject("SESSION_INDEPENDENCE_VIOLATION") { controller.verify(case_data) }
    end

    def test_inverted_timestamps
      case_data = build_base_case
      case_data["packet"]["producerCompletedAt"] = "2026-08-07T23:59:57.500Z"
      reseal!(case_data)
      expect_reject("NON_MONOTONIC_EVENT_CLOCK") { controller.verify(case_data) }
    end

    def test_success_artifact_policy
      case_data = build_base_case
      case_data["packet"]["artifacts"].pop
      paths = case_data["packet"]["artifacts"].map { |artifact| artifact["path"] }
      case_data["packet"]["artifactSetDigest"] = CanonicalJSON.sha256(paths.sort)
      reseal!(case_data)
      expect_reject("SUCCESS_ARTIFACT_SET_MISMATCH") { controller.verify(case_data) }
    end

    def test_generic_artifact_policy
      case_data = build_base_case
      case_data["packet"]["outcome"] = "BLOCKED"
      case_data["packet"]["branchCode"] = "BLOCK_MEASUREMENT_CONTRACT"
      case_data["packet"]["proposedNextEdges"] = ["R02->GK"]
      case_data["runtimeState"]["retryState"] = "EXHAUSTED"
      sync_generic_artifacts!(case_data, include_success_artifact: true)
      reseal!(case_data)
      attest_runtime!(case_data)
      result = controller.verify(case_data)
      Util.assert(result["effectiveEligibleNextEdges"] == ["R02->GK"], "SELF_TEST_GENERIC", "generic outcome did not reach only exhausted synthesis", result)

      missing = clone(case_data)
      required = ArtifactPolicy.new(materialized_manifest(missing), missing.dig("packet", "outcome"), missing.dig("packet", "branchCode")).required_paths.first
      missing["packet"]["artifacts"].reject! { |artifact| artifact["path"] == required }
      lookalike = add_artifact!(missing, required.sub(/failure\.json\z/, "failure-copy.json"), "wrong diagnostic path")
      missing["packet"]["artifacts"] << lookalike
      missing_paths = missing["packet"]["artifacts"].map { |artifact| artifact["path"] }
      missing["packet"]["artifactSetDigest"] = CanonicalJSON.sha256(missing_paths.sort)
      reseal!(missing)
      expect_reject("MISSING_GENERIC_DIAGNOSTIC") { controller.verify(missing) }

      attack = clone(case_data)
      undeclared = add_artifact!(attack, "undeclared/attack.json", "attack")
      attack["packet"]["artifacts"] << undeclared
      attack["packet"]["artifactSetDigest"] = CanonicalJSON.sha256(attack["packet"]["artifacts"].map { |artifact| artifact["path"] }.sort)
      reseal!(attack)
      expect_reject("UNDECLARED_PACKET_ARTIFACT") { controller.verify(attack) }
    end

    def test_self_artifacts
      missing = build_base_case
      missing["selfArtifacts"].delete("summary")
      expect_reject("SELF_ARTIFACT_SET_MISMATCH") { controller.verify(missing) }

      mismatch = build_base_case
      packet_ref = mismatch.dig("selfArtifacts", "packet")
      altered = clone(mismatch["packet"])
      altered["humanReadableConclusion"] = "different separately stored packet"
      mismatch["selfArtifacts"]["packet"] = add_object(
        mismatch["objectStore"], packet_ref["ref"], altered, "RFC8785_JSON"
      )
      expect_reject("PACKET_SELF_OBJECT_MISMATCH") { controller.verify(mismatch) }

      empty_summary = build_base_case
      summary_ref = empty_summary.dig("selfArtifacts", "summary")
      empty_summary["selfArtifacts"]["summary"] = add_object(
        empty_summary["objectStore"], summary_ref["ref"], "", "UTF8"
      )
      expect_reject("SUMMARY_SELF_OBJECT_EMPTY") { controller.verify(empty_summary) }
    end

    def test_evidence_reference_mutations
      dangling = build_base_case
      missing_ref = digest_ref("missing/evidence.json", "f" * 64)
      dangling["packet"]["claims"].first["evidenceRefs"] = [clone(missing_ref)]
      dangling["runtimeState"]["authorizedEvidenceRefs"] = [clone(missing_ref)]
      reseal!(dangling)
      attest_runtime!(dangling)
      expect_reject("DANGLING_DIGEST_REF") { controller.verify(dangling) }

      mismatch = build_base_case
      local_ref = mismatch.dig("packet", "claims", 0, "evidenceRefs", 0)
      local_ref["sha256"] = local_ref["sha256"] == ("f" * 64) ? ("e" * 64) : ("f" * 64)
      reseal!(mismatch)
      expect_reject("EVIDENCE_REF_DIGEST_MISMATCH") { controller.verify(mismatch) }
    end

    def test_token_domain
      case_data = build_base_case
      set_unavailable_telemetry!(case_data)
      result = controller.verify(case_data)
      Util.assert(result["effectiveEligibleNextEdges"] == ["R02->K01"], "SELF_TEST_TOKEN", "unavailable token telemetry should preserve semantic K01 edge")

      attack = clone(case_data)
      attack["packet"]["claims"].first["domains"] << "TOKEN"
      reseal!(attack)
      expect_reject("TOKEN_DOMAIN_FORBIDDEN") { controller.verify(attack) }
    end

    def test_available_metric_eligibility
      case_data = build_base_case
      case_data["packet"]["metricEligibility"]["nativeTokens"] = "UNAVAILABLE"
      reseal!(case_data)
      expect_reject("METRIC_ELIGIBILITY_MISMATCH") { controller.verify(case_data) }
    end

    def test_mixed_token_availability
      case_data = build_base_case
      set_verifier_token_unavailable!(case_data)
      result = controller.verify(case_data)
      Util.assert(result["budgetStatus"] == "TOKEN_TELEMETRY_UNAVAILABLE" &&
                  result["effectiveEligibleNextEdges"] == ["R02->K01"],
                  "SELF_TEST_MIXED_TOKEN_AVAILABILITY", "mixed producer/verifier telemetry projection is inconsistent", result)
    end

    def test_over_budget
      case_data = build_base_case
      set_over_budget!(case_data)
      result = controller.verify(case_data)
      Util.assert(result["effectiveOutcome"] == "NO_PROGRESS" && result["effectiveBranchCode"] == "BUDGET_EXCEEDED",
                  "SELF_TEST_OVER_BUDGET", "over-budget projection is wrong", result)
      Util.assert(result["effectiveEligibleNextEdges"] == ["R02->GK"], "SELF_TEST_OVER_BUDGET", "over-budget result unlocked a continuation", result)
    end

    def test_token_formula
      case_data = build_base_case
      case_data["packet"]["telemetry"]["noncachedTokens"] = 0
      case_data["receipt"]["costAccounting"]["teamTotals"]["noncachedTokens"] = 55
      reseal!(case_data)
      expect_reject("TOKEN_TELEMETRY_FORMULA_MISMATCH") { controller.verify(case_data) }
    end

    def promote_to_attempt_two!(case_data, validate_schema: false)
      node_id = case_data.dig("packet", "nodeId")
      current_packet = clone(case_data["packet"])
      current_receipt = clone(case_data["receipt"])
      current_runtime = clone(case_data["runtimeState"])

      case_data["packet"]["outcome"] = "BLOCKED"
      case_data["packet"]["branchCode"] = node_id == "R03" ? "REWORK_ARCHITECTURE" : "REWORK_EVIDENCE_SCHEMA"
      case_data["packet"]["proposedNextEdges"] = []
      case_data["runtimeState"]["retryState"] = "RETRY_ALLOWED"
      sync_generic_artifacts!(case_data)
      reseal!(case_data)
      attest_runtime!(case_data)
      prior_result = controller.verify(case_data)
      Util.assert(prior_result["controllerVerdict"] == "CONTROL_ACCEPT" && prior_result["effectiveEligibleNextEdges"] == [],
                  "SELF_TEST_RETRY", "retryable generic attempt 1 was not independently accepted", prior_result)
      if validate_schema
        ajv_validate(PACKET_SCHEMA_PATH, case_data["packet"], "#{node_id.downcase}-retryable-attempt-1-packet")
        ajv_validate(RECEIPT_SCHEMA_PATH, case_data["receipt"], "#{node_id.downcase}-retryable-attempt-1-receipt")
      end
      prior_packet = clone(case_data["packet"])
      prior_receipt = clone(case_data["receipt"])
      prior_packet_self_ref = clone(case_data.dig("selfArtifacts", "packet"))
      prior_summary_self_ref = clone(case_data.dig("selfArtifacts", "summary"))
      packet_ref = add_object(case_data["objectStore"], "attempts/#{node_id}/1/packet.json", prior_packet, "RFC8785_JSON")
      receipt_ref = add_object(case_data["objectStore"], "attempts/#{node_id}/1/receipt.json", prior_receipt, "RECEIPT_WITHOUT_RECEIPT_DIGEST")
      ancestry = [{
        "attempt" => 1,
        "packetRef" => packet_ref,
        "receiptRef" => receipt_ref,
        "packetSelfRef" => prior_packet_self_ref,
        "summarySelfRef" => prior_summary_self_ref
      }]

      case_data["packet"] = current_packet
      case_data["receipt"] = current_receipt
      case_data["runtimeState"] = current_runtime
      case_data["packet"]["attempt"] = 2
      case_data["packet"]["startedAt"] = "2026-08-08T00:00:10Z"
      case_data["packet"]["producerCompletedAt"] = "2026-08-08T00:00:11Z"
      case_data["bindings"]["ATTEMPT"] = "2"
      case_data["receipt"]["verifiedAt"] = "2026-08-08T00:00:11.500Z"
      case_data["receipt"]["costAccounting"]["priorAttemptReceipts"] = clone(ancestry)
      case_data["receipt"]["costAccounting"]["teamTotals"] = {
        "nativeTokenTelemetryAvailable" => true,
        "inputTokens" => 300,
        "cachedInputTokens" => 40,
        "outputTokens" => 30,
        "noncachedTokens" => 290,
        "toolCalls" => 12,
        "teamWallMilliseconds" => 13_000,
        "maxVisibleContextBytes" => 4096
      }
      case_data["runtimeState"]["attemptHistory"] = clone(ancestry)
      prior_manifest = case_data.fetch("manifestBundle").fetch("manifests").find { |manifest| manifest["id"] == node_id }
      case_data["runtimeState"]["retryAuthorization"] = RetryAuthorization.build(ancestry.first, prior_packet, prior_receipt, prior_manifest)
      sync_success_artifacts!(case_data)
      reseal!(case_data)
      attest_runtime!(case_data)
      case_data
    end

    def attach_r01_parent!(case_data, r01_case)
      add_object(case_data["objectStore"], "chain/R01/1/packet.json", r01_case["packet"], "RFC8785_JSON")
      parent_ref = add_object(case_data["objectStore"], "chain/R01/1/receipt.json", r01_case["receipt"], "RECEIPT_WITHOUT_RECEIPT_DIGEST")
      case_data["packet"]["parentReceiptDigests"] = [parent_ref]
      case_data["runtimeState"]["expectedParentReceiptDigests"] = [clone(parent_ref)]
      reseal!(case_data)
      attest_runtime!(case_data)
      case_data
    end

    def build_exhausted_attempt_two(node_id, r01_case)
      case_data = attach_r01_parent!(build_base_case(node_id), r01_case)
      promote_to_attempt_two!(case_data, validate_schema: true)
      case_data["packet"]["outcome"] = "NO_PROGRESS"
      case_data["packet"]["branchCode"] = node_id == "R03" ? "BLOCK_BASELINE_REGRESSION" : "BLOCK_MEASUREMENT_CONTRACT"
      case_data["packet"]["proposedNextEdges"] = ["#{node_id}->GK"]
      case_data["runtimeState"]["retryState"] = "EXHAUSTED"
      sync_generic_artifacts!(case_data)
      reseal!(case_data)
      attest_runtime!(case_data)
      ajv_validate(PACKET_SCHEMA_PATH, case_data["packet"], "#{node_id.downcase}-exhausted-attempt-2-packet")
      ajv_validate(RECEIPT_SCHEMA_PATH, case_data["receipt"], "#{node_id.downcase}-exhausted-attempt-2-receipt")
      result = controller.verify(case_data)
      Util.assert(result["effectiveEligibleNextEdges"] == ["#{node_id}->GK"], "SELF_TEST_EXHAUSTED_CHAIN",
                  "#{node_id} exhausted attempt 2 did not reach only GK", result)
      [case_data, result]
    end

    def replace_prior_pair!(case_data)
      ancestry = case_data.dig("receipt", "costAccounting", "priorAttemptReceipts").first
      packet_entry = case_data.fetch("objectStore").find { |entry| entry["ref"] == ancestry.dig("packetRef", "ref") }
      receipt_entry = case_data.fetch("objectStore").find { |entry| entry["ref"] == ancestry.dig("receiptRef", "ref") }
      prior_packet = clone(packet_entry.fetch("content"))
      prior_receipt = clone(receipt_entry.fetch("content"))
      yield prior_packet, prior_receipt
      prior_receipt["packetOutcome"] = prior_packet["outcome"]
      prior_receipt["packetBranchCode"] = prior_packet["branchCode"]
      prior_receipt["packetDigest"] = CanonicalJSON.sha256(prior_packet)
      prior_receipt["costAccounting"]["producerTelemetryDigest"] = CanonicalJSON.sha256(prior_packet["telemetry"])
      prior_receipt["receiptDigest"] = CanonicalJSON.sha256(Util.without_key(prior_receipt, "receiptDigest"))
      packet_ref = add_object(case_data["objectStore"], packet_entry["ref"], prior_packet, "RFC8785_JSON")
      receipt_ref = add_object(case_data["objectStore"], receipt_entry["ref"], prior_receipt, "RECEIPT_WITHOUT_RECEIPT_DIGEST")
      packet_self_ref = add_object(case_data["objectStore"], ancestry.dig("packetSelfRef", "ref"), prior_packet, "RFC8785_JSON")
      updated = [{
        "attempt" => 1,
        "packetRef" => packet_ref,
        "receiptRef" => receipt_ref,
        "packetSelfRef" => packet_self_ref,
        "summarySelfRef" => clone(ancestry["summarySelfRef"])
      }]
      case_data["receipt"]["costAccounting"]["priorAttemptReceipts"] = clone(updated)
      case_data["runtimeState"]["attemptHistory"] = clone(updated)
      node_id = prior_packet["nodeId"]
      manifest = case_data.fetch("manifestBundle").fetch("manifests").find { |item| item["id"] == node_id }
      case_data["runtimeState"]["retryAuthorization"] = RetryAuthorization.build(updated.first, prior_packet, prior_receipt, manifest)
      reseal!(case_data)
      attest_runtime!(case_data)
    end

    def test_retry_ancestry
      case_data = promote_to_attempt_two!(build_base_case)
      result = controller.verify(case_data)
      Util.assert(result["controllerVerdict"] == "CONTROL_ACCEPT", "SELF_TEST_RETRY", "valid attempt-2 ancestry was rejected")

      attack = clone(case_data)
      attack["receipt"]["costAccounting"]["priorAttemptReceipts"] = []
      reseal!(attack)
      expect_reject("RETRY_ANCESTRY_MISMATCH") { controller.verify(attack) }

      successful = promote_to_attempt_two!(build_base_case)
      replace_prior_pair!(successful) do |packet, _receipt|
        packet["outcome"] = "SUCCESS"
        packet["branchCode"] = "NONE"
      end
      expect_reject("PRIOR_ATTEMPT_NOT_RETRYABLE") { controller.verify(successful) }

      exhausted = promote_to_attempt_two!(build_base_case)
      ceiling = materialized_manifest(exhausted).dig("budgets", "noncachedTokenCeiling")
      replace_prior_pair!(exhausted) do |packet, receipt|
        producer_noncached = ceiling + 1
        packet["telemetry"].merge!(
          "inputTokens" => producer_noncached,
          "cachedInputTokens" => 0,
          "outputTokens" => 0,
          "noncachedTokens" => producer_noncached
        )
        verifier = receipt.dig("costAccounting", "verifierTelemetry")
        receipt["costAccounting"]["teamTotals"].merge!(
          "inputTokens" => producer_noncached + verifier["inputTokens"],
          "cachedInputTokens" => verifier["cachedInputTokens"],
          "outputTokens" => verifier["outputTokens"],
          "noncachedTokens" => producer_noncached + verifier["noncachedTokens"]
        )
        receipt["costAccounting"]["budgetStatus"] = "EXCEEDED"
        receipt["costAccounting"]["exceededMetrics"] = ["NONCACHED_TOKENS"]
      end
      expect_reject("PRIOR_ATTEMPT_NOT_RETRYABLE") { controller.verify(exhausted) }

      budget_branch = promote_to_attempt_two!(build_base_case)
      replace_prior_pair!(budget_branch) do |packet, _receipt|
        packet["branchCode"] = "BUDGET_EXCEEDED"
      end
      expect_reject("PRIOR_ATTEMPT_NOT_RETRYABLE") { controller.verify(budget_branch) }
    end

    def test_prior_receipt_digest
      case_data = promote_to_attempt_two!(build_base_case)
      prior_ref = case_data.dig("receipt", "costAccounting", "priorAttemptReceipts", 0, "receiptRef", "ref")
      stored = case_data.fetch("objectStore").find { |entry| entry["ref"] == prior_ref }
      stored.fetch("content")["receiptDigest"] = "f" * 64
      expect_reject("PRIOR_RECEIPT_DIGEST_MISMATCH") { controller.verify(case_data) }
    end

    def test_prior_artifact_closure
      artifact_tamper = promote_to_attempt_two!(build_base_case)
      replace_prior_pair!(artifact_tamper) do |packet, _receipt|
        packet["artifacts"] = []
        packet["artifactSetDigest"] = "0" * 64
      end
      expect_reject("MISSING_GENERIC_DIAGNOSTIC") { controller.verify(artifact_tamper) }

      self_ref_tamper = promote_to_attempt_two!(build_base_case)
      ancestry = clone(self_ref_tamper.dig("receipt", "costAccounting", "priorAttemptReceipts"))
      ancestry.first["packetSelfRef"] = clone(ancestry.first["summarySelfRef"])
      self_ref_tamper["receipt"]["costAccounting"]["priorAttemptReceipts"] = clone(ancestry)
      self_ref_tamper["runtimeState"]["attemptHistory"] = clone(ancestry)
      prior_packet = self_ref_tamper["objectStore"].find { |entry| entry["ref"] == ancestry.first.dig("packetRef", "ref") }["content"]
      prior_receipt = self_ref_tamper["objectStore"].find { |entry| entry["ref"] == ancestry.first.dig("receiptRef", "ref") }["content"]
      manifest = self_ref_tamper["manifestBundle"]["manifests"].find { |item| item["id"] == prior_packet["nodeId"] }
      self_ref_tamper["runtimeState"]["retryAuthorization"] = RetryAuthorization.build(
        ancestry.first, prior_packet, prior_receipt, manifest
      )
      reseal!(self_ref_tamper)
      attest_runtime!(self_ref_tamper)
      expect_reject("PRIOR_SELF_ARTIFACT_REF_MISMATCH") { controller.verify(self_ref_tamper) }
    end

    def test_effective_edges_authoritative
      case_data = build_base_case
      case_data["packet"]["proposedNextEdges"] = ["R02->ATTACK"]
      reseal!(case_data)
      result = controller.verify(case_data)
      Util.assert(result["effectiveEligibleNextEdges"] == ["R02->K01"], "SELF_TEST_EDGE_AUTHORITY", "producer hint changed effective edge", result)
      Util.assert(result["producerEdgeHintMatches"] == false, "SELF_TEST_EDGE_AUTHORITY", "false producer hint was reported as matching")

      injected = build_base_case
      row = clone(injected.dig("runtimeState", "edgeRegistry").find { |edge| edge["id"] == "R02->K01" })
      row["id"] = "R02->UNAPPROVED"
      row["target"] = "UNAPPROVED"
      injected["runtimeState"]["edgeRegistry"] << row
      attest_runtime!(injected)
      expect_reject("EDGE_REGISTRY_DOT_MISMATCH") { controller.verify(injected) }
    end

    def parent_pair(objects, node_id, attempt, suffix)
      telemetry = {
        "nativeTokenTelemetryAvailable" => true,
        "inputTokens" => 1,
        "cachedInputTokens" => 0,
        "outputTokens" => 0,
        "noncachedTokens" => 1,
        "toolCalls" => 1,
        "wallMilliseconds" => 10,
        "visibleContextBytes" => 128
      }
      packet = {
        "nodeId" => node_id,
        "attempt" => attempt,
        "startedAt" => "2026-08-07T23:50:00Z",
        "producerCompletedAt" => "2026-08-07T23:50:01Z",
        "outcome" => "NO_PROGRESS",
        "branchCode" => "BUDGET_EXCEEDED",
        "telemetry" => telemetry
      }
      receipt = {
        "nodeId" => node_id,
        "attempt" => attempt,
        "verdict" => "ACCEPT",
        "packetDigest" => CanonicalJSON.sha256(packet),
        "verifiedAt" => "2026-08-07T23:50:02Z",
        "receiptDigest" => "0" * 64
      }
      receipt["receiptDigest"] = CanonicalJSON.sha256(Util.without_key(receipt, "receiptDigest"))
      packet_ref = add_object(objects, "wave/#{node_id}/#{suffix}/packet.json", packet, "RFC8785_JSON")
      receipt_ref = add_object(objects, "wave/#{node_id}/#{suffix}/receipt.json", receipt, "RECEIPT_WITHOUT_RECEIPT_DIGEST")
      {
        "nodeId" => node_id,
        "packetRef" => packet_ref,
        "receiptRef" => receipt_ref,
        "accepted" => true,
        "exhausted" => true,
        "effectiveOutcome" => "NO_PROGRESS",
        "reachableContinuation" => false
      }
    end

    def build_gk_case
      r01_case = build_base_case("R01")
      r01_result = controller.verify(r01_case)
      Util.assert(r01_result["controllerVerdict"] == "CONTROL_ACCEPT", "SELF_TEST_EXHAUSTED_CHAIN", "R01 chain root was rejected")
      r02_case, r02_result = build_exhausted_attempt_two("R02", r01_case)
      r03_case, r03_result = build_exhausted_attempt_two("R03", r01_case)

      case_data = build_base_case("GK")
      make_parent = lambda do |source_case, result|
        node_id = source_case.dig("packet", "nodeId")
        packet_ref = add_object(case_data["objectStore"], "wave/#{node_id}/2/packet.json", source_case["packet"], "RFC8785_JSON")
        receipt_ref = add_object(case_data["objectStore"], "wave/#{node_id}/2/receipt.json", source_case["receipt"], "RECEIPT_WITHOUT_RECEIPT_DIGEST")
        {
          "nodeId" => node_id,
          "packetRef" => packet_ref,
          "receiptRef" => receipt_ref,
          "accepted" => true,
          "exhausted" => true,
          "effectiveOutcome" => result["effectiveOutcome"],
          "reachableContinuation" => false
        }
      end
      r02 = make_parent.call(r02_case, r02_result)
      r03 = make_parent.call(r03_case, r03_result)
      parents = [r02, r03]
      receipt_refs = parents.map { |parent| parent["receiptRef"] }.sort_by { |ref| [ref["ref"], ref["sha256"]] }
      case_data["packet"]["parentReceiptDigests"] = clone(receipt_refs)
      case_data["runtimeState"]["expectedParentReceiptDigests"] = clone(receipt_refs)
      case_data["runtimeState"]["gkWave"] = {
        "scope" => "FOUNDATION",
        "quiescent" => true,
        "normalContinuationReachable" => false,
        "exhaustedParents" => parents
      }
      case_data["runtimeState"]["edgeRegistry"] = DotEdgeRegistry.for_source(File.read(DOT_PATH), "GK")
      reseal!(case_data)
      attest_runtime!(case_data)
      case_data
    end

    def test_gk
      case_data = build_gk_case
      ajv_validate(PACKET_SCHEMA_PATH, case_data["packet"], "gk-quiescent-packet")
      ajv_validate(RECEIPT_SCHEMA_PATH, case_data["receipt"], "gk-quiescent-receipt")
      result = controller.verify(case_data)
      Util.assert(result["effectiveEligibleNextEdges"] == ["GK->GF0"], "SELF_TEST_GK", "GK exposed an implementation edge", result)

      missing = clone(case_data)
      missing["packet"]["parentReceiptDigests"].pop
      reseal!(missing)
      expect_reject("PARENT_RECEIPT_SET_MISMATCH") { controller.verify(missing) }

      not_exhausted = clone(case_data)
      not_exhausted["runtimeState"]["gkWave"]["exhaustedParents"].first["exhausted"] = false
      attest_runtime!(not_exhausted)
      expect_reject("GK_PARENT_NOT_EXHAUSTED") { controller.verify(not_exhausted) }

      active = clone(case_data)
      active["runtimeState"]["gkWave"]["quiescent"] = false
      attest_runtime!(active)
      expect_reject("GK_WAVE_NOT_QUIESCENT") { controller.verify(active) }

      wrong_branch = clone(case_data)
      wrong_branch["packet"]["branchCode"] = "NONE"
      reseal!(wrong_branch)
      expect_reject("GK_BOOTSTRAP_OUTPUT_INVALID") { controller.verify(wrong_branch) }
    end

    def convert_to_codex_mode!(case_data)
      case_data["mode"] = "CODEX_READ_ONLY_APPROVAL_V1"
      case_data["schemaPaths"] = {
        "approvalBundle" => APPROVAL_SCHEMA_PATH,
        "nodeContract" => NODE_SCHEMA_PATH,
        "packet" => PACKET_SCHEMA_PATH,
        "receipt" => RECEIPT_SCHEMA_PATH
      }
      case_data
    end

    def test_codex_read_only_mode
      case_data = convert_to_codex_mode!(build_base_case)
      verifier = Controller.new(self_test: false)
      result = verifier.verify(case_data)
      Util.assert(result["controllerVerdict"] == "CONTROL_ACCEPT", "SELF_TEST_CODEX_MODE", "valid Codex observations were rejected")

      attack = clone(case_data)
      attack.dig("runtimeAttestation", "observations", 1)["observerSessionId"] =
        attack.dig("runtimeAttestation", "observations", 0, "observerSessionId")
      observation = attack.dig("runtimeAttestation", "observations", 1)
      observation["receiptDigest"] = CanonicalJSON.sha256(Util.without_key(observation, "receiptDigest"))
      expect_reject("RUNTIME_OBSERVER_INDEPENDENCE_VIOLATION") { verifier.verify(attack) }

      collision = clone(case_data)
      runtime_observation = collision.dig("runtimeAttestation", "observations", 0)
      runtime_observation["observerSessionId"] = collision.dig("packet", "producer", "sessionId")
      runtime_observation["receiptDigest"] = CanonicalJSON.sha256(Util.without_key(runtime_observation, "receiptDigest"))
      expect_reject("AUTHORITY_SESSION_INDEPENDENCE_VIOLATION") { verifier.verify(collision) }
    end
  end
end

if $PROGRAM_NAME == __FILE__
  options = {}
  parser = OptionParser.new do |commands|
    commands.banner = "Usage: #{File.basename(__FILE__)} --self-test | --verify CASE.json"
    commands.on("--self-test", "Run deterministic planning-side bootstrap verification") { options[:self_test] = true }
    commands.on("--verify FILE", "Verify one runtime controller case") { |value| options[:verify] = value }
    commands.on("-h", "--help", "Show this help") do
      puts commands
      exit 0
    end
  end

  begin
    parser.parse!(ARGV)
    if options[:self_test] == !!options[:verify]
      raise OptionParser::InvalidArgument, "choose exactly one of --self-test or --verify FILE"
    end
    unless ARGV.empty?
      raise OptionParser::InvalidArgument, "unexpected arguments: #{ARGV.join(' ')}"
    end

    if options[:self_test]
      report = CodeclewBootstrapV0::SelfTest.new.run
      puts JSON.pretty_generate(report)
      exit(report["status"] == "PASS" ? 0 : 1)
    end

    case_data = JSON.parse(File.read(options.fetch(:verify)))
    controller = CodeclewBootstrapV0::Controller.new(self_test: false)
    puts JSON.pretty_generate(controller.verify(case_data))
  rescue CodeclewBootstrapV0::Reject => error
    puts JSON.pretty_generate(
      "controllerVerdict" => "CONTROL_REJECT",
      "rejectCode" => error.code,
      "message" => error.message,
      "details" => error.details
    )
    exit 2
  rescue OptionParser::ParseError => error
    warn "#{error.message}\n#{parser}"
    exit 64
  rescue JSON::ParserError, Errno::ENOENT, OpenSSL::PKey::PKeyError => error
    puts JSON.pretty_generate(
      "controllerVerdict" => "CONTROL_ERROR",
      "error" => error.class.name,
      "message" => error.message
    )
    exit 3
  end
end
