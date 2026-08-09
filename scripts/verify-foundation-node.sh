#!/usr/bin/env bash
set -u

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
SCOPE=JQ_1_7_SORTED_COMPACT_INTEGER_JSON
RECEIPT_SCOPE=JQ_1_7_SORTED_COMPACT_INTEGER_JSON_WITHOUT_RECEIPT_DIGEST
EXPECTED_PLAN_CONTRACT_DIGEST=cd50b780bdf5a0ea04f2ae48fa3a44523743760f43268fd3ecd7d8cb2eb2f5a3
SELF_TEST_MODE=0

sha_file() { shasum -a 256 "$1" | awk '{print $1}'; }
sha_stream() { shasum -a 256 | awk '{print $1}'; }
integer_json() { jq -e '[.. | numbers] | all(. == floor)' "$1" >/dev/null; }
canonical_digest() { integer_json "$1" && jq -cS . "$1" | sha_stream; }
canonical_filter_digest() { integer_json "$2" && jq -cS "$1" "$2" | sha_stream; }
reject() { printf '%s\t%s\n' "$1" "$2" >&2; exit 2; }
require_file() { [ -f "$1" ] || reject DANGLING_REF "missing file: $1"; }
valid_sha() { [[ $1 =~ ^[a-f0-9]{64}$ ]]; }

repo_path() {
  case "$1" in
    /*|*../*|../*|*/..) reject INVALID_ARTIFACT_REF "unsafe repository path: $1" ;;
  esac
  printf '%s/%s' "$ROOT" "$1"
}

resolved_ref_path() {
  if [[ $1 = /* ]]; then
    [ "$SELF_TEST_MODE" -eq 1 ] || reject INVALID_ARTIFACT_REF "absolute ref is TEST_ONLY: $1"
    printf '%s' "$1"
  else
    repo_path "$1"
  fi
}

check_schema_documents() {
  local schema
  for schema in \
    "$ROOT/schemas/evidence/foundation-approval-v1.schema.json" \
    "$ROOT/schemas/evidence/foundation-packet-v1.schema.json" \
    "$ROOT/schemas/evidence/foundation-receipt-v1.schema.json"; do
    require_file "$schema"
    jq -e '."$schema" == "https://json-schema.org/draft/2020-12/schema" and .type == "object" and .additionalProperties == false and (.required|type == "array" and length > 0)' "$schema" >/dev/null \
      || reject INVALID_SCHEMA_DOCUMENT "schema structure invalid: $schema"
    integer_json "$schema" || reject FLOAT_NOT_SUPPORTED "schema contains a non-integer number"
  done
}

check_manifest() {
  local manifest=$1 plan=$2 expected actual role path
  jq -e --arg scope "$SCOPE" 'def exact($k): (keys|sort)==($k|sort); exact(["schemaVersion","canonicalScope","plan","sidecars","historicalTuple","sharedSubjects","sharedDigests","nodes","retryPolicy","outcomeRows","b03Execution","planContractDigest"]) and .schemaVersion == "codeclew-optimized-foundation-manifests/1" and .canonicalScope == $scope and (.sidecars|length == 4) and (.nodes|length == 4) and (.outcomeRows|length == 22) and (.b03Execution|length == 7)' "$manifest" >/dev/null \
    || reject INVALID_MANIFEST "manifest structure or exact row count invalid"
  integer_json "$manifest" || reject FLOAT_NOT_SUPPORTED "manifest contains a non-integer number"
  actual=$(sha_file "$plan")
  expected=$(jq -r '.plan.rawFileSha256' "$manifest")
  [ "$actual" = "$expected" ] || reject STALE_PLAN_DIGEST "plan raw SHA does not match manifest"
  while IFS=$'\t' read -r role path expected; do
    local resolved
    resolved=$(repo_path "$path")
    require_file "$resolved"
    [ "$(sha_file "$resolved")" = "$expected" ] || reject SIDECAR_DIGEST_MISMATCH "$role raw SHA mismatch"
  done < <(jq -r '.sidecars[] | [.role,.path,.rawFileSha256] | @tsv' "$manifest")
  for role in source model topology budgetPolicy; do
    expected=$(jq -r --arg role "$role" '.sharedDigests[$role]' "$manifest")
    actual=$(canonical_filter_digest ".sharedSubjects[\"$role\"]" "$manifest")
    [ "$actual" = "$expected" ] || reject SHARED_DIGEST_MISMATCH "$role subject digest mismatch"
  done
  expected=$(canonical_filter_digest '.historicalTuple' "$manifest")
  [ "$(jq -r '.sharedSubjects.source.historicalTupleCanonicalDigest' "$manifest")" = "$expected" ] \
    || reject SHARED_DIGEST_MISMATCH "source subject does not bind historical tuple"
  actual=$(canonical_filter_digest '{nodes,outcomeRows,b03Execution}' "$manifest")
  [ "$(jq -r '.planContractDigest' "$manifest")" = "$actual" ] && [ "$actual" = "$EXPECTED_PLAN_CONTRACT_DIGEST" ] \
    || reject PLAN_CONTRACT_DIGEST_MISMATCH "approved budgets/outcome rows/B03 execution contract changed"
  while IFS=$'\t' read -r role expected; do
    actual=$(canonical_filter_digest ".nodes[] | select(.id == \"$role\") | .budget" "$manifest")
    [ "$actual" = "$expected" ] || reject NODE_BUDGET_DIGEST_MISMATCH "$role budget digest mismatch"
  done < <(jq -r '.nodes[] | [.id,.budgetDigest] | @tsv' "$manifest")
  jq -e '([.nodes[].id] | sort == ["B01","B02","B03","GB"]) and ([.b03Execution[].id] | sort == ["B03-Q1","B03-Q2","B03-Q3","B03-T1","B03-U1","B03-W1","B03-W2"]) and ([.outcomeRows | group_by([.nodeId,.outcome,.branchCode])[] | length] | all(. == 1))' "$manifest" >/dev/null \
    || reject INVALID_MANIFEST "node set or outcome row key is not exact"
}

check_approval() {
  local approval=$1 plan=$2 manifest=$3 subject expected path role
  jq -e --arg scope "$SCOPE" 'def exact($k): (keys|sort)==($k|sort); exact(["schemaVersion","planStatus","approvalSubject","approvalSubjectDigest","humanDecision","createdAt","canonicalScope"]) and .schemaVersion == "codeclew-foundation-approval/1" and .planStatus == "PROPOSED_AWAITING_HUMAN_APPROVAL" and .humanDecision == "HUMAN_APPROVED" and .canonicalScope == $scope and (.approvalSubject|exact(["plan","manifest","sidecars","historicalTuple","currentTaskEvent"])) and (.approvalSubject.plan|exact(["role","path","rawFileSha256"])) and (.approvalSubject.manifest|exact(["role","path","rawFileSha256"])) and ([.approvalSubject.sidecars[]|exact(["role","path","rawFileSha256"])]|all) and (.approvalSubject.currentTaskEvent|exact(["mode","taskId","messageId","authorRole","messageDigest"])) and (.approvalSubject.sidecars|length == 4)' "$approval" >/dev/null \
    || reject INVALID_APPROVAL "approval structure invalid"
  integer_json "$approval" || reject FLOAT_NOT_SUPPORTED "approval contains a non-integer number"
  expected=$(canonical_filter_digest '.approvalSubject' "$approval")
  [ "$(jq -r '.approvalSubjectDigest' "$approval")" = "$expected" ] || reject APPROVAL_SUBJECT_DIGEST_MISMATCH "approval subject digest mismatch"
  [ "$(jq -r '.approvalSubject.plan.rawFileSha256' "$approval")" = "$(sha_file "$plan")" ] || reject APPROVAL_PLAN_DIGEST_MISMATCH "approval plan SHA mismatch"
  [ "$(jq -r '.approvalSubject.manifest.rawFileSha256' "$approval")" = "$(sha_file "$manifest")" ] || reject APPROVAL_MANIFEST_DIGEST_MISMATCH "approval manifest SHA mismatch"
  jq -e --arg planPath "${plan#$ROOT/}" --arg manifestPath "${manifest#$ROOT/}" '.approvalSubject.plan.role == "PLAN" and .approvalSubject.plan.path == $planPath and .approvalSubject.manifest.role == "FOUNDATION_MANIFEST" and .approvalSubject.manifest.path == $manifestPath' "$approval" >/dev/null \
    || reject APPROVAL_ARTIFACT_REF_MISMATCH "approval plan/manifest refs are not exact"
  jq -e --slurpfile manifest "$manifest" '.approvalSubject.sidecars == $manifest[0].sidecars' "$approval" >/dev/null \
    || reject APPROVAL_ARTIFACT_REF_MISMATCH "approval sidecar refs differ from manifest"
  jq -e --slurpfile manifest "$manifest" '.approvalSubject.historicalTuple == $manifest[0].historicalTuple' "$approval" >/dev/null \
    || reject APPROVAL_HISTORICAL_TUPLE_MISMATCH "approval historical tuple mismatch"
  while IFS=$'\t' read -r role path expected; do
    local resolved
    resolved=$(repo_path "$path"); require_file "$resolved"
    [ "$(sha_file "$resolved")" = "$expected" ] || reject APPROVAL_SIDECAR_DIGEST_MISMATCH "$role SHA mismatch"
  done < <(jq -r '.approvalSubject.sidecars[] | [.role,.path,.rawFileSha256] | @tsv' "$approval")
  local mode
  mode=$(jq -r '.approvalSubject.currentTaskEvent.mode' "$approval")
  if [ "$mode" = TEST_ONLY ]; then
    [ "$SELF_TEST_MODE" -eq 1 ] || reject TEST_ONLY_FORBIDDEN "TEST_ONLY approval is allowed only inside --self-test"
  else
    [ "$mode" = NORMAL ] || reject INVALID_APPROVAL_EVENT "approval event mode invalid"
    jq -e '.approvalSubject.currentTaskEvent | .taskId != "" and .messageId != "" and .authorRole == "USER" and (.messageDigest|test("^[a-f0-9]{64}$"))' "$approval" >/dev/null \
      || reject INVALID_APPROVAL_EVENT "current task approval event is incomplete"
  fi
}

check_artifacts() {
  local packet=$1 path expected resolved
  while IFS=$'\t' read -r path expected; do
    resolved=$(repo_path "$path"); require_file "$resolved"
    [ "$(sha_file "$resolved")" = "$expected" ] || reject ARTIFACT_DIGEST_MISMATCH "artifact raw SHA mismatch: $path"
  done < <(jq -r '.artifactRefs[] | [.path,.rawFileSha256] | @tsv' "$packet")
}

check_packet_shape() {
  jq -e --arg scope "$SCOPE" '
    def exact($k): (keys|sort)==($k|sort);
    def nni: type=="number" and .>=0 and .==floor;
    exact(["schemaVersion","nodeId","attempt","outcome","branchCode","producer","planRawFileSha256","approvalRawFileSha256","manifestRawFileSha256","sharedDigests","budgetDigest","artifactRefs","parentReceipts","telemetry","retryAncestry","proposedEdges","humanReadableConclusion","canonicalScope"]) and
    .schemaVersion=="codeclew-foundation-packet/1" and .canonicalScope==$scope and
    (.nodeId|test("^(B01|B02|B03|GB)$")) and (.attempt|nni) and (.attempt==1 or .attempt==2) and
    (.producer|exact(["agentId","sessionId"])) and (.producer.agentId|type=="string" and length>0) and (.producer.sessionId|type=="string" and length>0) and
    (.sharedDigests|exact(["source","model","topology","budgetPolicy"])) and
    ([.planRawFileSha256,.approvalRawFileSha256,.manifestRawFileSha256,.budgetDigest,.sharedDigests[]]|all(type=="string" and test("^[a-f0-9]{64}$"))) and
    ([.artifactRefs[]|exact(["path","rawFileSha256"]) and (.path|type=="string" and length>0) and (.rawFileSha256|test("^[a-f0-9]{64}$"))]|all) and
    ([.parentReceipts[]|exact(["nodeId","packetRef","receiptRef"])]|all) and
    ([.parentReceipts[].packetRef,.parentReceipts[].receiptRef|exact(["path","rawFileSha256"]) and (.path|type=="string" and length>0) and (.rawFileSha256|test("^[a-f0-9]{64}$"))]|all) and
    (.telemetry|exact(["nativeTokenTelemetryAvailable","inputTokens","cachedInputTokens","outputTokens","noncachedTokens","actionCalls","waitCalls","chargedCalls","wallMilliseconds","maxVisibleContextBytes"])) and
    (.telemetry.nativeTokenTelemetryAvailable|type=="boolean") and
    ([.telemetry.actionCalls,.telemetry.waitCalls,.telemetry.chargedCalls,.telemetry.wallMilliseconds,.telemetry.maxVisibleContextBytes]|all(nni)) and
    (.proposedEdges|type=="array") and (.artifactRefs|type=="array") and (.parentReceipts|type=="array") and
    (.humanReadableConclusion|type=="string" and length>0)
  ' "$1" >/dev/null || reject INVALID_PACKET "packet key set, type, or nonnegative integer constraint invalid"
}

check_receipt_shape() {
  jq -e --arg scope "$SCOPE" --arg receiptScope "$RECEIPT_SCOPE" '
    def exact($k): (keys|sort)==($k|sort);
    def nni: type=="number" and .>=0 and .==floor;
    exact(["schemaVersion","nodeId","attempt","packetRawFileSha256","packetCanonicalDigest","approvalRawFileSha256","manifestRawFileSha256","planRawFileSha256","sharedDigests","budgetDigest","producerSessionId","verifier","independenceAttestation","checks","verdict","packetOutcome","packetBranchCode","costAccounting","verifiedAt","canonicalScope","receiptDigestScope","receiptDigest"]) and
    .schemaVersion=="codeclew-foundation-receipt/1" and .canonicalScope==$scope and .receiptDigestScope==$receiptScope and
    (.attempt|nni) and (.attempt==1 or .attempt==2) and
    (.sharedDigests|exact(["source","model","topology","budgetPolicy"])) and
    ([.packetRawFileSha256,.packetCanonicalDigest,.approvalRawFileSha256,.manifestRawFileSha256,.planRawFileSha256,.budgetDigest,.receiptDigest,.sharedDigests[]]|all(type=="string" and test("^[a-f0-9]{64}$"))) and
    (.verifier|exact(["agentId","sessionId"])) and (.verifier.agentId|type=="string" and length>0) and (.verifier.sessionId|type=="string" and length>0) and
    (.costAccounting|exact(["nativeTokenTelemetryAvailable","inputTokens","cachedInputTokens","outputTokens","noncachedTokens","actionCalls","waitCalls","chargedCalls","wallMilliseconds","maxVisibleContextBytes"])) and
    ([.costAccounting.actionCalls,.costAccounting.waitCalls,.costAccounting.chargedCalls,.costAccounting.wallMilliseconds,.costAccounting.maxVisibleContextBytes]|all(nni)) and
    ([.checks[]|exact(["checkId","result"]) and (.checkId|type=="string" and length>0) and (.result|type=="string")]|all) and (.checks|length>0) and ([.checks[].result]|all(.=="PASS")) and
    (.producerSessionId|type=="string" and length>0) and (.verifiedAt|type=="string" and length>0) and
    .verdict=="ACCEPT" and .independenceAttestation==true
  ' "$1" >/dev/null || reject INVALID_RECEIPT "receipt key set, type, or nonnegative integer constraint invalid"
}

validate_parent_pair() {
  local manifest=$1 gb_packet=$2 ref=$3 expected_node=$4 packet_path receipt_path expected actual outcome branch budget
  packet_path=$(resolved_ref_path "$(jq -r '.packetRef.path' <<<"$ref")")
  receipt_path=$(resolved_ref_path "$(jq -r '.receiptRef.path' <<<"$ref")")
  require_file "$packet_path"; require_file "$receipt_path"
  [ "$(sha_file "$packet_path")" = "$(jq -r '.packetRef.rawFileSha256' <<<"$ref")" ] || reject GB_PARENT_DIGEST_MISMATCH "$expected_node packet ref mismatch"
  [ "$(sha_file "$receipt_path")" = "$(jq -r '.receiptRef.rawFileSha256' <<<"$ref")" ] || reject GB_PARENT_DIGEST_MISMATCH "$expected_node receipt ref mismatch"
  check_packet_shape "$packet_path"; check_receipt_shape "$receipt_path"
  [ "$(jq -r '.nodeId' "$packet_path")" = "$expected_node" ] && [ "$(jq -r '.nodeId' "$receipt_path")" = "$expected_node" ] || reject GB_PARENT_NODE_MISMATCH "$expected_node parent identity mismatch"
  [ "$(jq -r '.verdict' "$receipt_path")" = ACCEPT ] || reject GB_PARENT_NOT_ACCEPTED "$expected_node receipt not accepted"
  [ "$(sha_file "$packet_path")" = "$(jq -r '.packetRawFileSha256' "$receipt_path")" ] || reject GB_PARENT_DIGEST_MISMATCH "$expected_node receipt packet raw mismatch"
  [ "$(canonical_digest "$packet_path")" = "$(jq -r '.packetCanonicalDigest' "$receipt_path")" ] || reject GB_PARENT_DIGEST_MISMATCH "$expected_node receipt packet canonical mismatch"
  [ "$(canonical_filter_digest 'del(.receiptDigest)' "$receipt_path")" = "$(jq -r '.receiptDigest' "$receipt_path")" ] || reject GB_PARENT_DIGEST_MISMATCH "$expected_node receipt digest mismatch"
  jq -e --slurpfile packet "$packet_path" '.packetOutcome==$packet[0].outcome and .packetBranchCode==$packet[0].branchCode and .sharedDigests==$packet[0].sharedDigests and .budgetDigest==$packet[0].budgetDigest and .planRawFileSha256==$packet[0].planRawFileSha256 and .approvalRawFileSha256==$packet[0].approvalRawFileSha256 and .manifestRawFileSha256==$packet[0].manifestRawFileSha256 and .producerSessionId==$packet[0].producer.sessionId' "$receipt_path" >/dev/null || reject GB_PARENT_IDENTITY_MISMATCH "$expected_node packet/receipt mismatch"
  [ "$(jq -r '.verifier.agentId' "$receipt_path")" != "$(jq -r '.producer.agentId' "$packet_path")" ] && [ "$(jq -r '.verifier.sessionId' "$receipt_path")" != "$(jq -r '.producer.sessionId' "$packet_path")" ] || reject GB_PARENT_NOT_ACCEPTED "$expected_node receipt is not independent"
  jq -e --slurpfile gb "$gb_packet" '.sharedDigests==$gb[0].sharedDigests and .planRawFileSha256==$gb[0].planRawFileSha256 and .approvalRawFileSha256==$gb[0].approvalRawFileSha256 and .manifestRawFileSha256==$gb[0].manifestRawFileSha256' "$packet_path" >/dev/null || reject GB_PARENT_PARITY_MISMATCH "$expected_node plan/source/model/topology/budget-policy parity mismatch"
  expected=$(jq -r --arg node "$expected_node" '.nodes[]|select(.id==$node)|.budgetDigest' "$manifest")
  [ "$(jq -r '.budgetDigest' "$receipt_path")" = "$expected" ] || reject GB_PARENT_BUDGET_MISMATCH "$expected_node budget mismatch"
  outcome=$(jq -r '.outcome' "$packet_path"); branch=$(jq -r '.branchCode' "$packet_path")
  [ "$(jq --arg node "$expected_node" --arg outcome "$outcome" --arg branch "$branch" '[.outcomeRows[]|select(.nodeId==$node and .outcome==$outcome and .branchCode==$branch)]|length' "$manifest")" -eq 1 ] || reject GB_PARENT_BRANCH_INVALID "$expected_node branch invalid"
  printf '%s+%s' "$outcome" "$branch"
}

derive_gb_branch() {
  local manifest=$1 packet=$2 b02_ref b03_ref b02 b03
  [ "$(jq '.parentReceipts|length' "$packet")" -eq 2 ] || reject GB_PARENT_SET_MISMATCH "GB requires exactly B02 and B03 parent receipts"
  [ "$(jq '[.parentReceipts[].nodeId]|sort==["B02","B03"]' "$packet")" = true ] || reject GB_PARENT_SET_MISMATCH "GB parent set must be exactly B02/B03"
  b02_ref=$(jq -c '.parentReceipts[]|select(.nodeId=="B02")' "$packet"); b03_ref=$(jq -c '.parentReceipts[]|select(.nodeId=="B03")' "$packet")
  b02=$(validate_parent_pair "$manifest" "$packet" "$b02_ref" B02)
  b03=$(validate_parent_pair "$manifest" "$packet" "$b03_ref" B03)
  case "$b02|$b03" in
    'SUCCESS+NONE|SUCCESS+NONE') printf NONE ;;
    'SUCCESS+TOKEN_TELEMETRY_UNAVAILABLE|SUCCESS+NONE') printf TOKEN_CLAIMS_UNAVAILABLE ;;
    'SUCCESS+NONE|SUCCESS+NARROW_BASELINE_CONTOUR') printf NARROW_BASELINE_CONTOUR ;;
    'SUCCESS+TOKEN_TELEMETRY_UNAVAILABLE|SUCCESS+NARROW_BASELINE_CONTOUR') printf NARROW_BASELINE_AND_TOKEN_CLAIMS_UNAVAILABLE ;;
    *) printf INCONCLUSIVE_FOUNDATION ;;
  esac
}

validate_retry_ancestry() {
  local manifest=$1 packet=$2 node=$3 charged=$4 ancestry prior_packet prior_receipt expected initial retry remaining outcome branch
  ancestry=$(jq -c '.retryAncestry' "$packet")
  jq -e 'def exact($k):(keys|sort)==($k|sort); exact(["priorPacketRef","priorReceiptRef","acceptedRetryable","priorAttempt","priorOutcome","priorBranchCode","failureFingerprint","changedPaths","changedInvariants","initialAttemptChargedCalls","retryChargedCalls","remainingChargedCalls"]) and (.priorPacketRef|exact(["path","rawFileSha256"])) and (.priorReceiptRef|exact(["path","rawFileSha256"])) and .acceptedRetryable==true and .priorAttempt==1 and (.failureFingerprint|test("^[a-f0-9]{64}$")) and (.changedPaths|type=="array" and length>0) and (.changedInvariants|type=="array" and length>0)' <<<"$ancestry" >/dev/null || reject BAD_RETRY_ANCESTRY "attempt 2 ancestry structure invalid"
  prior_packet=$(resolved_ref_path "$(jq -r '.priorPacketRef.path' <<<"$ancestry")"); prior_receipt=$(resolved_ref_path "$(jq -r '.priorReceiptRef.path' <<<"$ancestry")")
  require_file "$prior_packet"; require_file "$prior_receipt"
  [ "$(sha_file "$prior_packet")" = "$(jq -r '.priorPacketRef.rawFileSha256' <<<"$ancestry")" ] && [ "$(sha_file "$prior_receipt")" = "$(jq -r '.priorReceiptRef.rawFileSha256' <<<"$ancestry")" ] || reject BAD_RETRY_ANCESTRY "prior raw digests mismatch"
  check_packet_shape "$prior_packet"; check_receipt_shape "$prior_receipt"
  jq -e --arg node "$node" --slurpfile prior "$prior_packet" '.nodeId==$node and .attempt==1 and .verdict=="ACCEPT" and .packetOutcome==$prior[0].outcome and .packetBranchCode==$prior[0].branchCode and .receiptDigest==(.receiptDigest)' "$prior_receipt" >/dev/null || reject BAD_RETRY_ANCESTRY "prior packet/receipt is not accepted attempt 1"
  [ "$(sha_file "$prior_packet")" = "$(jq -r '.packetRawFileSha256' "$prior_receipt")" ] && [ "$(canonical_digest "$prior_packet")" = "$(jq -r '.packetCanonicalDigest' "$prior_receipt")" ] && [ "$(canonical_filter_digest 'del(.receiptDigest)' "$prior_receipt")" = "$(jq -r '.receiptDigest' "$prior_receipt")" ] || reject BAD_RETRY_ANCESTRY "prior receipt integrity invalid"
  outcome=$(jq -r '.outcome' "$prior_packet"); branch=$(jq -r '.branchCode' "$prior_packet")
  [ "$outcome" = "$(jq -r '.priorOutcome' <<<"$ancestry")" ] && [ "$branch" = "$(jq -r '.priorBranchCode' <<<"$ancestry")" ] || reject BAD_RETRY_ANCESTRY "prior outcome/branch declaration mismatch"
  [ "$(jq --arg node "$node" --arg outcome "$outcome" --arg branch "$branch" '[.retryPolicy.retryableRows[]|select(.nodeId==$node and .outcome==$outcome and .branchCode==$branch)]|length' "$manifest")" -eq 1 ] || reject BAD_RETRY_ANCESTRY "prior branch is not retryable"
  expected=$(canonical_filter_digest '{nodeId,outcome,branchCode,humanReadableConclusion}' "$prior_packet")
  [ "$(jq -r '.failureFingerprint' <<<"$ancestry")" = "$expected" ] || reject BAD_RETRY_ANCESTRY "failure fingerprint does not bind prior failure"
  initial=$(jq -r '.initialAttemptChargedCalls' <<<"$ancestry"); retry=$(jq -r '.retryChargedCalls' <<<"$ancestry"); remaining=$(jq -r '.remainingChargedCalls' <<<"$ancestry")
  [[ $initial =~ ^[0-9]+$ && $retry =~ ^[0-9]+$ && $remaining =~ ^[0-9]+$ ]] || reject BAD_RETRY_ANCESTRY "retry counters must be nonnegative integers"
  [ "$initial" -eq "$(jq -r '.telemetry.chargedCalls' "$prior_packet")" ] && [ "$retry" -eq "$charged" ] && [ "$retry" -le $((initial*30/100)) ] && [ "$retry" -le "$remaining" ] || reject BAD_RETRY_ANCESTRY "retry call binding or ceiling invalid"
}

verify_impl() {
  local plan=$1 approval=$2 manifest=$3 requested_node=$4 packet=$5 receipt=$6
  local node attempt outcome branch expected actual row_count budget charged action wait available input cached output noncached wall context
  check_schema_documents
  require_file "$plan"; require_file "$approval"; require_file "$manifest"; require_file "$packet"; require_file "$receipt"
  integer_json "$packet" && integer_json "$receipt" || reject FLOAT_NOT_SUPPORTED "packet or receipt contains non-integer number"
  check_manifest "$manifest" "$plan"
  check_approval "$approval" "$plan" "$manifest"
  check_packet_shape "$packet"
  check_receipt_shape "$receipt"
  node=$(jq -r '.nodeId' "$packet"); attempt=$(jq -r '.attempt' "$packet"); outcome=$(jq -r '.outcome' "$packet"); branch=$(jq -r '.branchCode' "$packet")
  [ "$node" = "$requested_node" ] || reject NODE_ID_MISMATCH "requested node differs from packet"
  if [ "$node" = GB ]; then
    local derived_branch
    derived_branch=$(derive_gb_branch "$manifest" "$packet")
    [ "$outcome" = SUCCESS ] && [ "$branch" = "$derived_branch" ] || reject GB_DERIVATION_MISMATCH "GB branch must be mechanically derived from B02/B03 parents"
  else
    [ "$(jq '.parentReceipts|length' "$packet")" -eq 0 ] || reject GB_PARENT_SET_MISMATCH "non-GB packets cannot carry GB parents"
  fi
  row_count=$(jq --arg node "$node" --arg outcome "$outcome" --arg branch "$branch" '[.outcomeRows[] | select(.nodeId==$node and .outcome==$outcome and .branchCode==$branch)] | length' "$manifest")
  [ "$row_count" -eq 1 ] || reject ILLEGAL_OUTCOME_BRANCH "outcome/branch pair is not authorized"
  jq -e --arg node "$node" --arg outcome "$outcome" --arg branch "$branch" --slurpfile packet "$packet" '(.outcomeRows[] | select(.nodeId==$node and .outcome==$outcome and .branchCode==$branch) | (.eligibleEdges|sort)) == ($packet[0].proposedEdges|sort)' "$manifest" >/dev/null \
    || reject UNAUTHORIZED_EDGE "packet edges differ from exact manifest row"
  [ "$(jq -r '.planRawFileSha256' "$packet")" = "$(sha_file "$plan")" ] || reject STALE_PLAN_DIGEST "packet plan SHA mismatch"
  [ "$(jq -r '.approvalRawFileSha256' "$packet")" = "$(sha_file "$approval")" ] || reject APPROVAL_DIGEST_MISMATCH "packet approval raw SHA mismatch"
  [ "$(jq -r '.manifestRawFileSha256' "$packet")" = "$(sha_file "$manifest")" ] || reject MANIFEST_DIGEST_MISMATCH "packet manifest raw SHA mismatch"
  jq -e --slurpfile manifest "$manifest" '.sharedDigests == $manifest[0].sharedDigests' "$packet" >/dev/null || reject SHARED_DIGEST_MISMATCH "packet shared digests mismatch"
  expected=$(jq -r --arg node "$node" '.nodes[] | select(.id==$node) | .budgetDigest' "$manifest")
  [ "$(jq -r '.budgetDigest' "$packet")" = "$expected" ] || reject NODE_BUDGET_DIGEST_MISMATCH "packet node budget digest mismatch"
  check_artifacts "$packet"
  action=$(jq -r '.telemetry.actionCalls' "$packet"); wait=$(jq -r '.telemetry.waitCalls' "$packet"); charged=$(jq -r '.telemetry.chargedCalls' "$packet")
  [ $((action + wait)) -eq "$charged" ] || reject CHARGED_CALL_FORMULA_MISMATCH "actionCalls + waitCalls != chargedCalls"
  available=$(jq -r '.telemetry.nativeTokenTelemetryAvailable' "$packet")
  if [ "$available" = true ]; then
    input=$(jq -r '.telemetry.inputTokens' "$packet"); cached=$(jq -r '.telemetry.cachedInputTokens' "$packet"); output=$(jq -r '.telemetry.outputTokens' "$packet"); noncached=$(jq -r '.telemetry.noncachedTokens' "$packet")
    [[ $input =~ ^[0-9]+$ && $cached =~ ^[0-9]+$ && $output =~ ^[0-9]+$ && $noncached =~ ^[0-9]+$ ]] || reject TOKEN_TELEMETRY_INVALID "available token fields must be integers"
    [ "$cached" -le "$input" ] && [ $((input - cached + output)) -eq "$noncached" ] || reject TOKEN_FORMULA_MISMATCH "noncached token formula mismatch"
  else
    jq -e '[.telemetry.inputTokens,.telemetry.cachedInputTokens,.telemetry.outputTokens,.telemetry.noncachedTokens] | all(. == null)' "$packet" >/dev/null \
      || reject TOKEN_UNAVAILABLE_FIELDS_NOT_NULL "unavailable token fields must be null"
  fi
  budget=$(jq -c --arg node "$node" '.nodes[] | select(.id==$node) | .budget' "$manifest")
  wall=$(jq -r '.telemetry.wallMilliseconds' "$packet"); context=$(jq -r '.telemetry.maxVisibleContextBytes' "$packet")
  [ "$charged" -le "$(jq -r '.chargedCallCeiling' <<<"$budget")" ] && [ "$wall" -le "$(jq -r '.wallMillisecondsCeiling' <<<"$budget")" ] && [ "$context" -le "$(jq -r '.maxVisibleContextBytes' <<<"$budget")" ] \
    || reject BUDGET_EXCEEDED "non-token budget ceiling exceeded"
  if [ "$available" = true ]; then
    [ "$noncached" -le "$(jq -r '.noncachedTokenCeiling' <<<"$budget")" ] && [ "$output" -le "$(jq -r '.outputTokenCeiling' <<<"$budget")" ] \
      || reject BUDGET_EXCEEDED "token budget ceiling exceeded"
  fi
  if [ "$attempt" -eq 1 ]; then
    jq -e '.retryAncestry == null' "$packet" >/dev/null || reject BAD_RETRY_ANCESTRY "attempt 1 cannot have retry ancestry"
  else
    validate_retry_ancestry "$manifest" "$packet" "$node" "$charged"
  fi
  expected=$(sha_file "$packet"); [ "$(jq -r '.packetRawFileSha256' "$receipt")" = "$expected" ] || reject PACKET_RAW_DIGEST_MISMATCH "packet raw SHA mismatch"
  expected=$(canonical_digest "$packet"); [ "$(jq -r '.packetCanonicalDigest' "$receipt")" = "$expected" ] || reject PACKET_CANONICAL_DIGEST_MISMATCH "packet canonical digest mismatch"
  expected=$(canonical_filter_digest 'del(.receiptDigest)' "$receipt"); [ "$(jq -r '.receiptDigest' "$receipt")" = "$expected" ] || reject RECEIPT_DIGEST_MISMATCH "receipt canonical digest mismatch"
  jq -e --slurpfile packet "$packet" '.nodeId==$packet[0].nodeId and .attempt==$packet[0].attempt and .packetOutcome==$packet[0].outcome and .packetBranchCode==$packet[0].branchCode and .producerSessionId==$packet[0].producer.sessionId and .planRawFileSha256==$packet[0].planRawFileSha256 and .approvalRawFileSha256==$packet[0].approvalRawFileSha256 and .manifestRawFileSha256==$packet[0].manifestRawFileSha256 and .sharedDigests==$packet[0].sharedDigests and .budgetDigest==$packet[0].budgetDigest and .costAccounting==$packet[0].telemetry' "$receipt" >/dev/null \
    || reject PACKET_RECEIPT_IDENTITY_MISMATCH "receipt does not mirror packet identity/cost"
  [ "$(jq -r '.verifier.sessionId' "$receipt")" != "$(jq -r '.producer.sessionId' "$packet")" ] && [ "$(jq -r '.verifier.agentId' "$receipt")" != "$(jq -r '.producer.agentId' "$packet")" ] || reject NON_INDEPENDENT_VERIFIER "producer and verifier agent IDs and sessions must both differ"
  jq -cn --arg node "$node" --argjson attempt "$attempt" --arg outcome "$outcome" --arg branch "$branch" --slurpfile manifest "$manifest" '{controllerVerdict:"CONTROL_ACCEPT",nodeId:$node,attempt:$attempt,effectiveOutcome:$outcome,effectiveBranchCode:$branch,effectiveEligibleNextEdges:($manifest[0].outcomeRows[]|select(.nodeId==$node and .outcome==$outcome and .branchCode==$branch)|.eligibleEdges),canonicalScope:"JQ_1_7_SORTED_COMPACT_INTEGER_JSON"}'
}

publish_result() {
  local plan=$1 node=$2 attempt=$3 source=$4
  local digest base dir target temp
  digest=$(sha_file "$plan")
  base=${FOUNDATION_RESULT_ROOT:-"$ROOT/evidence/graphs"}
  dir="$base/$digest/controller/$node/$attempt"; mkdir -p "$dir"
  target="$dir/result.json"; temp="$dir/.result.$$"
  cp "$source" "$temp" && mv "$temp" "$target"
}

run_case() {
  local plan=$1 approval=$2 manifest=$3 node=$4 packet=$5 receipt=$6 out err rc attempt
  out=$(mktemp); err=$(mktemp)
  (verify_impl "$plan" "$approval" "$manifest" "$node" "$packet" "$receipt") >"$out" 2>"$err"; rc=$?
  if [ "$rc" -eq 0 ]; then
    (check_artifacts "$packet") 2>"$err" || rc=$?
  fi
  if [ "$rc" -eq 0 ]; then :; elif [ "$rc" -eq 2 ]; then
    local code message
    IFS=$'\t' read -r code message <"$err"
    jq -cn --arg code "$code" --arg message "$message" '{controllerVerdict:"CONTROL_REJECT",rejectCode:$code,message:$message}' >"$out"
  else
    jq -cn --arg message "$(tr '\n' ' ' <"$err")" '{controllerVerdict:"CONTROL_ERROR",error:"INFRA_ERROR",message:$message}' >"$out"; rc=3
  fi
  attempt=$(jq -r '.attempt // 1' "$packet" 2>/dev/null || printf 1)
  if ! publish_result "$plan" "$node" "$attempt" "$out" 2>"$err"; then
    jq -cn --arg message "$(tr '\n' ' ' <"$err")" '{controllerVerdict:"CONTROL_ERROR",error:"PUBLICATION_FAILED",message:$message}' >"$out"
    cat "$out"; rm -f "$out" "$err"; return 3
  fi
  cat "$out"
  rm -f "$out" "$err"
  return "$rc"
}

make_fixture() {
  local dir=$1 node=$2 outcome=$3 branch=$4 edges=$5 plan=$6 manifest=$7 approval packet receipt subject_digest packet_raw packet_canonical receipt_digest budget_digest
  approval="$dir/approval.json"; packet="$dir/$node-packet.json"; receipt="$dir/$node-receipt.json"
  jq -n --slurpfile manifest "$manifest" --arg planPath "${plan#$ROOT/}" --arg planSha "$(sha_file "$plan")" --arg manifestPath "${manifest#$ROOT/}" --arg manifestSha "$(sha_file "$manifest")" '{schemaVersion:"codeclew-foundation-approval/1",planStatus:"PROPOSED_AWAITING_HUMAN_APPROVAL",approvalSubject:{plan:{role:"PLAN",path:$planPath,rawFileSha256:$planSha},manifest:{role:"FOUNDATION_MANIFEST",path:$manifestPath,rawFileSha256:$manifestSha},sidecars:$manifest[0].sidecars,historicalTuple:$manifest[0].historicalTuple,currentTaskEvent:{mode:"TEST_ONLY",taskId:"foundation-self-test",messageId:"self-test-approval",authorRole:"USER",messageDigest:"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}},approvalSubjectDigest:"",humanDecision:"HUMAN_APPROVED",createdAt:"2026-08-09T00:00:00Z",canonicalScope:"JQ_1_7_SORTED_COMPACT_INTEGER_JSON"}' >"$approval"
  subject_digest=$(canonical_filter_digest '.approvalSubject' "$approval"); jq --arg digest "$subject_digest" '.approvalSubjectDigest=$digest' "$approval" >"$approval.tmp" && mv "$approval.tmp" "$approval"
  budget_digest=$(jq -r --arg node "$node" '.nodes[]|select(.id==$node)|.budgetDigest' "$manifest")
  jq -n --slurpfile manifest "$manifest" --arg node "$node" --arg outcome "$outcome" --arg branch "$branch" --argjson edges "$edges" --arg planSha "$(sha_file "$plan")" --arg approvalSha "$(sha_file "$approval")" --arg manifestSha "$(sha_file "$manifest")" --arg budgetDigest "$budget_digest" '{schemaVersion:"codeclew-foundation-packet/1",nodeId:$node,attempt:1,outcome:$outcome,branchCode:$branch,producer:{agentId:"producer-agent",sessionId:"producer-session"},planRawFileSha256:$planSha,approvalRawFileSha256:$approvalSha,manifestRawFileSha256:$manifestSha,sharedDigests:$manifest[0].sharedDigests,budgetDigest:$budgetDigest,artifactRefs:[],parentReceipts:[],telemetry:{nativeTokenTelemetryAvailable:false,inputTokens:null,cachedInputTokens:null,outputTokens:null,noncachedTokens:null,actionCalls:2,waitCalls:1,chargedCalls:3,wallMilliseconds:1000,maxVisibleContextBytes:1024},retryAncestry:null,proposedEdges:$edges,humanReadableConclusion:"self-test positive fixture",canonicalScope:"JQ_1_7_SORTED_COMPACT_INTEGER_JSON"}' >"$packet"
  packet_raw=$(sha_file "$packet"); packet_canonical=$(canonical_digest "$packet")
  jq -n --slurpfile packet "$packet" --arg packetRaw "$packet_raw" --arg packetCanonical "$packet_canonical" '{schemaVersion:"codeclew-foundation-receipt/1",nodeId:$packet[0].nodeId,attempt:$packet[0].attempt,packetRawFileSha256:$packetRaw,packetCanonicalDigest:$packetCanonical,approvalRawFileSha256:$packet[0].approvalRawFileSha256,manifestRawFileSha256:$packet[0].manifestRawFileSha256,planRawFileSha256:$packet[0].planRawFileSha256,sharedDigests:$packet[0].sharedDigests,budgetDigest:$packet[0].budgetDigest,producerSessionId:$packet[0].producer.sessionId,verifier:{agentId:"verifier-agent",sessionId:"verifier-session"},independenceAttestation:true,checks:[{checkId:"SELF_TEST",result:"PASS"}],verdict:"ACCEPT",packetOutcome:$packet[0].outcome,packetBranchCode:$packet[0].branchCode,costAccounting:$packet[0].telemetry,verifiedAt:"2026-08-09T00:00:01Z",canonicalScope:"JQ_1_7_SORTED_COMPACT_INTEGER_JSON",receiptDigestScope:"JQ_1_7_SORTED_COMPACT_INTEGER_JSON_WITHOUT_RECEIPT_DIGEST",receiptDigest:""}' >"$receipt"
  receipt_digest=$(canonical_filter_digest 'del(.receiptDigest)' "$receipt"); jq --arg digest "$receipt_digest" '.receiptDigest=$digest' "$receipt" >"$receipt.tmp" && mv "$receipt.tmp" "$receipt"
  printf '%s\t%s\t%s\n' "$approval" "$packet" "$receipt"
}

reseal_receipt() {
  local packet=$1 receipt=$2 digest
  jq --arg raw "$(sha_file "$packet")" --arg canonical "$(canonical_digest "$packet")" --slurpfile packet "$packet" '.packetRawFileSha256=$raw | .packetCanonicalDigest=$canonical | .nodeId=$packet[0].nodeId | .attempt=$packet[0].attempt | .approvalRawFileSha256=$packet[0].approvalRawFileSha256 | .manifestRawFileSha256=$packet[0].manifestRawFileSha256 | .planRawFileSha256=$packet[0].planRawFileSha256 | .sharedDigests=$packet[0].sharedDigests | .budgetDigest=$packet[0].budgetDigest | .producerSessionId=$packet[0].producer.sessionId | .packetOutcome=$packet[0].outcome | .packetBranchCode=$packet[0].branchCode | .costAccounting=$packet[0].telemetry | .receiptDigest=""' "$receipt" >"$receipt.tmp" && mv "$receipt.tmp" "$receipt"
  digest=$(canonical_filter_digest 'del(.receiptDigest)' "$receipt"); jq --arg digest "$digest" '.receiptDigest=$digest' "$receipt" >"$receipt.tmp" && mv "$receipt.tmp" "$receipt"
}

attach_gb_parents() {
  local packet=$1 receipt=$2 b02_packet=$3 b02_receipt=$4 b03_packet=$5 b03_receipt=$6
  jq --arg b02p "$b02_packet" --arg b02ps "$(sha_file "$b02_packet")" --arg b02r "$b02_receipt" --arg b02rs "$(sha_file "$b02_receipt")" --arg b03p "$b03_packet" --arg b03ps "$(sha_file "$b03_packet")" --arg b03r "$b03_receipt" --arg b03rs "$(sha_file "$b03_receipt")" '.parentReceipts=[{nodeId:"B02",packetRef:{path:$b02p,rawFileSha256:$b02ps},receiptRef:{path:$b02r,rawFileSha256:$b02rs}},{nodeId:"B03",packetRef:{path:$b03p,rawFileSha256:$b03ps},receiptRef:{path:$b03r,rawFileSha256:$b03rs}}]' "$packet" >"$packet.tmp" && mv "$packet.tmp" "$packet"
  reseal_receipt "$packet" "$receipt"
}

self_test() {
  SELF_TEST_MODE=1
  local dir plan manifest approval packet receipt node tuple out rc pass=0 total=0
  dir=$(mktemp -d); export FOUNDATION_RESULT_ROOT="$dir/results"
  plan="$ROOT/docs/superpowers/plans/2026-08-09-codeclew-optimized-research-foundation-plan.md"
  manifest="$ROOT/docs/superpowers/plans/codeclew-optimized-foundation-manifests-v1.json"
  for tuple in 'B01 SUCCESS NONE ["B01->B02","B01->B03"]' 'B02 SUCCESS NONE ["B02->GB"]' 'B03 SUCCESS NONE ["B03->GB"]'; do
    read -r node _ _ _ <<<"$tuple"; set -- $tuple; node=$1
    IFS=$'\t' read -r approval packet receipt < <(make_fixture "$dir" "$1" "$2" "$3" "$4" "$plan" "$manifest")
    total=$((total+1)); out=$(run_case "$plan" "$approval" "$manifest" "$node" "$packet" "$receipt"); rc=$?
    [ "$rc" -eq 0 ] && [ "$(jq -r '.controllerVerdict' <<<"$out")" = CONTROL_ACCEPT ] && pass=$((pass+1)) || return 1
  done
  gb_positive() {
    local b02_branch=$1 b03_branch=$2 gb_branch=$3 b02a b02p b02r b03a b03p b03r gba gbp gbr
    IFS=$'\t' read -r b02a b02p b02r < <(make_fixture "$dir" B02 SUCCESS "$b02_branch" '["B02->GB"]' "$plan" "$manifest")
    IFS=$'\t' read -r b03a b03p b03r < <(make_fixture "$dir" B03 SUCCESS "$b03_branch" '["B03->GB"]' "$plan" "$manifest")
    IFS=$'\t' read -r gba gbp gbr < <(make_fixture "$dir" GB SUCCESS "$gb_branch" '["GB->K01"]' "$plan" "$manifest")
    attach_gb_parents "$gbp" "$gbr" "$b02p" "$b02r" "$b03p" "$b03r"
    total=$((total+1)); out=$(run_case "$plan" "$gba" "$manifest" GB "$gbp" "$gbr"); rc=$?
    [ "$rc" -eq 0 ] && [ "$(jq -r '.effectiveBranchCode' <<<"$out")" = "$gb_branch" ] && pass=$((pass+1)) || return 1
  }
  gb_positive NONE NONE NONE
  gb_positive TOKEN_TELEMETRY_UNAVAILABLE NONE TOKEN_CLAIMS_UNAVAILABLE
  gb_positive NONE NARROW_BASELINE_CONTOUR NARROW_BASELINE_CONTOUR
  gb_positive TOKEN_TELEMETRY_UNAVAILABLE NARROW_BASELINE_CONTOUR NARROW_BASELINE_AND_TOKEN_CLAIMS_UNAVAILABLE
  local prior_a prior_p prior_r retry_a retry_p retry_r fingerprint
  IFS=$'\t' read -r prior_a prior_p prior_r < <(make_fixture "$dir" B02 BLOCKED BLOCK_MEASUREMENT_CONTRACT '["B02->GF0"]' "$plan" "$manifest")
  jq '.telemetry.actionCalls=10|.telemetry.waitCalls=0|.telemetry.chargedCalls=10' "$prior_p" >"$prior_p.tmp" && mv "$prior_p.tmp" "$prior_p"; reseal_receipt "$prior_p" "$prior_r"
  cp "$prior_p" "$dir/prior-B02-packet.json"; cp "$prior_r" "$dir/prior-B02-receipt.json"; prior_p="$dir/prior-B02-packet.json"; prior_r="$dir/prior-B02-receipt.json"
  IFS=$'\t' read -r retry_a retry_p retry_r < <(make_fixture "$dir" B02 SUCCESS NONE '["B02->GB"]' "$plan" "$manifest")
  fingerprint=$(canonical_filter_digest '{nodeId,outcome,branchCode,humanReadableConclusion}' "$prior_p")
  jq --arg pp "$prior_p" --arg pps "$(sha_file "$prior_p")" --arg pr "$prior_r" --arg prs "$(sha_file "$prior_r")" --arg fingerprint "$fingerprint" '.attempt=2|.retryAncestry={priorPacketRef:{path:$pp,rawFileSha256:$pps},priorReceiptRef:{path:$pr,rawFileSha256:$prs},acceptedRetryable:true,priorAttempt:1,priorOutcome:"BLOCKED",priorBranchCode:"BLOCK_MEASUREMENT_CONTRACT",failureFingerprint:$fingerprint,changedPaths:["measurement-contract.json"],changedInvariants:["token-telemetry-policy"],initialAttemptChargedCalls:10,retryChargedCalls:3,remainingChargedCalls:20}' "$retry_p" >"$retry_p.tmp" && mv "$retry_p.tmp" "$retry_p"; reseal_receipt "$retry_p" "$retry_r"
  total=$((total+1)); out=$(run_case "$plan" "$retry_a" "$manifest" B02 "$retry_p" "$retry_r"); rc=$?; [ "$rc" -eq 0 ] && pass=$((pass+1)) || return 1
  IFS=$'\t' read -r approval packet receipt < <(make_fixture "$dir" B02 SUCCESS NONE '["B02->GB"]' "$plan" "$manifest")
  mutation() {
    local name=$1 code=$2 jq_packet=${3:-.} jq_receipt=${4:-.} local_plan=${5:-$plan}
    local p="$dir/m-$name-packet.json" r="$dir/m-$name-receipt.json" a="$approval"
    jq "$jq_packet" "$packet" >"$p"; jq "$jq_receipt" "$receipt" >"$r"; reseal_receipt "$p" "$r"
    if [ "$name" = bad-receipt-digest ]; then
      jq '.receiptDigest="dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"' "$r" >"$r.tmp" && mv "$r.tmp" "$r"
    fi
    total=$((total+1)); set +e; out=$(run_case "$local_plan" "$a" "$manifest" B02 "$p" "$r"); rc=$?; set -e
    if [ "$rc" -eq 2 ] && [ "$(jq -r '.rejectCode' <<<"$out")" = "$code" ]; then pass=$((pass+1)); else printf 'self-test %s expected %s got %s\n' "$name" "$code" "$out" >&2; return 1; fi
  }
  local stale="$dir/stale-plan.md"; cp "$plan" "$stale"; printf '\n' >>"$stale"
  mutation stale-plan STALE_PLAN_DIGEST '.' '.' "$stale"
  mutation wrong-budget NODE_BUDGET_DIGEST_MISMATCH '.budgetDigest="bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"'
  mutation illegal-branch ILLEGAL_OUTCOME_BRANCH '.branchCode="ILLEGAL"' '.packetBranchCode="ILLEGAL"'
  mutation non-independent NON_INDEPENDENT_VERIFIER '.' '.verifier.sessionId="producer-session"'
  mutation dangling-ref DANGLING_REF '.artifactRefs=[{path:"missing/artifact",rawFileSha256:"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}]'
  mutation over-budget BUDGET_EXCEEDED '.telemetry.actionCalls=31|.telemetry.waitCalls=0|.telemetry.chargedCalls=31' '.costAccounting.actionCalls=31|.costAccounting.waitCalls=0|.costAccounting.chargedCalls=31'
  mutation unauthorized-edge UNAUTHORIZED_EDGE '.proposedEdges=["B02->GF0"]'
  mutation bad-receipt-digest RECEIPT_DIGEST_MISMATCH '.' '.receiptDigest="dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"'
  mutation bad-retry BAD_RETRY_ANCESTRY '.attempt=2|.retryAncestry={acceptedRetryable:true,priorAttempt:1,priorOutcome:"BLOCKED",priorBranchCode:"BLOCK_MEASUREMENT_CONTRACT",failureFingerprint:"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",changedPaths:["x"],changedInvariants:[],initialAttemptChargedCalls:3,retryChargedCalls:3,remainingChargedCalls:3,priorPacketRef:{path:"missing",rawFileSha256:"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"},priorReceiptRef:{path:"missing",rawFileSha256:"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"}}' '.attempt=2'
  mutation extra-field INVALID_PACKET '.unexpected=true'
  mutation negative-telemetry INVALID_PACKET '.telemetry.actionCalls=-1|.telemetry.chargedCalls=0'

  total=$((total+1)); SELF_TEST_MODE=0; set +e; out=$(run_case "$plan" "$approval" "$manifest" B02 "$packet" "$receipt"); rc=$?; set -e; SELF_TEST_MODE=1
  [ "$rc" -eq 2 ] && [ "$(jq -r '.rejectCode' <<<"$out")" = TEST_ONLY_FORBIDDEN ] && pass=$((pass+1)) || return 1

  local pr="$dir/packet-raw-receipt.json"; cp "$receipt" "$pr"; jq '.packetRawFileSha256="ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"|.receiptDigest=""' "$pr" >"$pr.tmp" && mv "$pr.tmp" "$pr"; local d; d=$(canonical_filter_digest 'del(.receiptDigest)' "$pr"); jq --arg d "$d" '.receiptDigest=$d' "$pr" >"$pr.tmp" && mv "$pr.tmp" "$pr"
  total=$((total+1)); set +e; out=$(run_case "$plan" "$approval" "$manifest" B02 "$packet" "$pr"); rc=$?; set -e; [ "$rc" -eq 2 ] && [ "$(jq -r '.rejectCode' <<<"$out")" = PACKET_RAW_DIGEST_MISMATCH ] && pass=$((pass+1)) || return 1
  local pc="$dir/packet-canonical-receipt.json"; cp "$receipt" "$pc"; jq '.packetCanonicalDigest="ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"|.receiptDigest=""' "$pc" >"$pc.tmp" && mv "$pc.tmp" "$pc"; d=$(canonical_filter_digest 'del(.receiptDigest)' "$pc"); jq --arg d "$d" '.receiptDigest=$d' "$pc" >"$pc.tmp" && mv "$pc.tmp" "$pc"
  total=$((total+1)); set +e; out=$(run_case "$plan" "$approval" "$manifest" B02 "$packet" "$pc"); rc=$?; set -e; [ "$rc" -eq 2 ] && [ "$(jq -r '.rejectCode' <<<"$out")" = PACKET_CANONICAL_DIGEST_MISMATCH ] && pass=$((pass+1)) || return 1

  local gba gbp gbr b02a b02p b02r b03a b03p b03r
  IFS=$'\t' read -r gba gbp gbr < <(make_fixture "$dir" GB SUCCESS NONE '["GB->K01"]' "$plan" "$manifest")
  total=$((total+1)); set +e; out=$(run_case "$plan" "$gba" "$manifest" GB "$gbp" "$gbr"); rc=$?; set -e; [ "$rc" -eq 2 ] && [ "$(jq -r '.rejectCode' <<<"$out")" = GB_PARENT_SET_MISMATCH ] && pass=$((pass+1)) || return 1
  IFS=$'\t' read -r b02a b02p b02r < <(make_fixture "$dir" B02 SUCCESS NONE '["B02->GB"]' "$plan" "$manifest"); IFS=$'\t' read -r b03a b03p b03r < <(make_fixture "$dir" B03 SUCCESS NONE '["B03->GB"]' "$plan" "$manifest"); attach_gb_parents "$gbp" "$gbr" "$b02p" "$b02r" "$b03p" "$b03r"; jq '.parentReceipts[0].packetRef.rawFileSha256="ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"' "$gbp" >"$gbp.tmp" && mv "$gbp.tmp" "$gbp"; reseal_receipt "$gbp" "$gbr"
  total=$((total+1)); set +e; out=$(run_case "$plan" "$gba" "$manifest" GB "$gbp" "$gbr"); rc=$?; set -e; [ "$rc" -eq 2 ] && [ "$(jq -r '.rejectCode' <<<"$out")" = GB_PARENT_DIGEST_MISMATCH ] && pass=$((pass+1)) || return 1

  local blocked_root="$dir/not-a-directory"; printf x >"$blocked_root"; local saved_root=$FOUNDATION_RESULT_ROOT; export FOUNDATION_RESULT_ROOT="$blocked_root"
  total=$((total+1)); set +e; out=$(run_case "$plan" "$approval" "$manifest" B02 "$packet" "$receipt"); rc=$?; set -e; export FOUNDATION_RESULT_ROOT="$saved_root"; [ "$rc" -eq 3 ] && [ "$(jq -r '.error' <<<"$out")" = PUBLICATION_FAILED ] && pass=$((pass+1)) || return 1

  jq -cn --argjson positives 8 --argjson negatives 17 --argjson total "$total" --argjson passed "$pass" '{schemaVersion:"codeclew-foundation-self-test/1",status:(if $total==$passed then "PASS" else "FAIL" end),positiveCases:$positives,negativeCases:$negatives,total:$total,passed:$passed,canonicalScope:"JQ_1_7_SORTED_COMPACT_INTEGER_JSON"}'
  rm -rf "$dir"
  [ "$total" -eq "$pass" ]
}

usage() { printf 'Usage: %s --self-test | --plan FILE --approval FILE --manifest FILE --node NODE --packet FILE --receipt FILE\n' "${0##*/}" >&2; exit 64; }

if [ "${1:-}" = --self-test ]; then
  [ "$#" -eq 1 ] || usage
  self_test
  exit $?
fi

plan= approval= manifest= node= packet= receipt=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --plan|--approval|--manifest|--node|--packet|--receipt) [ "$#" -ge 2 ] || usage; key=${1#--}; printf -v "$key" '%s' "$2"; shift 2 ;;
    *) usage ;;
  esac
done
[ -n "$plan" ] && [ -n "$approval" ] && [ -n "$manifest" ] && [ -n "$node" ] && [ -n "$packet" ] && [ -n "$receipt" ] || usage
for variable in plan approval manifest packet receipt; do value=${!variable}; case "$value" in /*) ;; *) printf -v "$variable" '%s/%s' "$ROOT" "$value" ;; esac; done
set +e
run_case "$plan" "$approval" "$manifest" "$node" "$packet" "$receipt"
exit $?
