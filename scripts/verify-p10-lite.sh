#!/usr/bin/env bash
set -u

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
SCOPE=JQ_1_7_SORTED_COMPACT_INTEGER_JSON
BASE_PLAN_SHA=80f2b7308c0e4eb51c6376931591dc389d0c08e6d7dc75a4ab757b7395506a34
REJECTED_REPORT_SHA=65431d7bc58401e09f9733c14c4106fde6270d07fac0b49043664ad588f322fa
PROTOTYPE_SHA=0a6bf73f5a1fd795272c76afc19dc35d59106cae29c8e6395ab03487cfeee539
PLAN_CONTRACT_SHA=cd50b780bdf5a0ea04f2ae48fa3a44523743760f43268fd3ecd7d8cb2eb2f5a3
SELF_TEST=0

sha_file() { shasum -a 256 "$1" | awk '{print $1}'; }
sha_stream() { shasum -a 256 | awk '{print $1}'; }
integer_json() { jq -e '[.. | numbers] | all(. == floor)' "$1" >/dev/null; }
canonical_digest() { integer_json "$1" && jq -cS . "$1" | sha_stream; }
reject() { jq -cnS --arg code "$1" --arg message "$2" '{controllerVerdict:"CONTROL_REJECT",rejectCode:$code,message:$message}'; exit 2; }
infra() { jq -cnS --arg code "$1" --arg message "$2" '{controllerVerdict:"CONTROL_ERROR",error:$code,message:$message}'; exit 3; }
usage() { printf 'Usage: %s --self-test | --plan FILE --amendment FILE --manifest FILE --approval FILE\n' "${0##*/}" >&2; exit 64; }

require_file() { [ -f "$1" ] || infra MISSING_FILE "$1"; }
repo_path() {
  case "$1" in /*|*../*|../*|*/..) reject UNSAFE_PATH "$1" ;; esac
  printf '%s/%s' "$ROOT" "$1"
}

exact_manifest() {
  jq -e --arg scope "$SCOPE" '
    keys == ["amendment","approvalArtifactRoles","basePlan","canonicalScope","controller","deferredOwners","eligibleEdge","frozenRuntimePrototype","rejectedP10Evidence","schemaVersion","status"] and
    .schemaVersion=="codeclew-p10-lite-manifest/1" and
    .status=="PROPOSED_AWAITING_HUMAN_APPROVAL" and .canonicalScope==$scope and
    .basePlan.role=="BASE_PLAN" and .basePlan.rawFileSha256=="80f2b7308c0e4eb51c6376931591dc389d0c08e6d7dc75a4ab757b7395506a34" and
    .amendment.role=="P10_LITE_AMENDMENT" and
    .controller=={"role":"P10_LITE_CONTROLLER","path":"scripts/verify-p10-lite.sh"} and
    .rejectedP10Evidence.rawFileSha256=="65431d7bc58401e09f9733c14c4106fde6270d07fac0b49043664ad588f322fa" and
    .frozenRuntimePrototype.rawFileSha256=="0a6bf73f5a1fd795272c76afc19dc35d59106cae29c8e6395ab03487cfeee539" and
    .frozenRuntimePrototype.planContractDigest=="cd50b780bdf5a0ea04f2ae48fa3a44523743760f43268fd3ecd7d8cb2eb2f5a3" and
    .approvalArtifactRoles==["BASE_PLAN","P10_LITE_AMENDMENT","P10_LITE_MANIFEST","P10_LITE_CONTROLLER","REJECTED_P10_EVIDENCE","FROZEN_RUNTIME_PROTOTYPE"] and
    .eligibleEdge=="A10->B01" and
    .deferredOwners=={"packetReceiptSchema":"B02","parentJoin":"GB","parentRetryToctou":"B02_AND_GB","retryAccounting":"B02","tokenTelemetryCoupling":"B02"}
  ' "$1" >/dev/null || reject MANIFEST_CONTRACT_MISMATCH "lite manifest differs from amendment contract"
}

verify_package() {
  local plan=$1 amendment=$2 manifest=$3 approval=$4
  require_file "$plan"; require_file "$amendment"; require_file "$manifest"; require_file "$approval"
  integer_json "$manifest" || reject FLOAT_NOT_SUPPORTED manifest
  integer_json "$approval" || reject FLOAT_NOT_SUPPORTED approval
  exact_manifest "$manifest"
  [ "$(sha_file "$plan")" = "$BASE_PLAN_SHA" ] || reject STALE_BASE_PLAN "$plan"
  [ "$(sha_file "$plan")" = "$(jq -r '.basePlan.rawFileSha256' "$manifest")" ] || reject STALE_BASE_PLAN manifest
  [ "$(sha_file "$amendment")" = "$(jq -r '.amendment.rawFileSha256' "$manifest")" ] || reject STALE_AMENDMENT manifest
  local report prototype
  report=$(repo_path "$(jq -r '.rejectedP10Evidence.path' "$manifest")"); require_file "$report"
  prototype=$(repo_path "$(jq -r '.frozenRuntimePrototype.path' "$manifest")"); require_file "$prototype"
  [ "$(sha_file "$report")" = "$REJECTED_REPORT_SHA" ] || reject STALE_REJECTED_EVIDENCE "$report"
  [ "$(sha_file "$prototype")" = "$PROTOTYPE_SHA" ] || reject STALE_RUNTIME_PROTOTYPE "$prototype"
  [ "$(jq -r '.planContractDigest' "$prototype")" = "$PLAN_CONTRACT_SHA" ] || reject PLAN_CONTRACT_MISMATCH prototype

  jq -e --arg scope "$SCOPE" '
    keys==["approvalSubject","approvalSubjectDigest","canonicalScope","createdAt","humanDecision","schemaVersion"] and
    .schemaVersion=="codeclew-p10-lite-approval/1" and .humanDecision=="HUMAN_APPROVED" and
    .canonicalScope==$scope and (.createdAt|type=="string" and length>0) and
    (.approvalSubject|keys)==["artifacts","currentTaskEvent"] and
    (.approvalSubject.artifacts|type=="array") and
    (.approvalSubject.currentTaskEvent|keys)==["authorRole","messageDigest","messageId","mode","taskId"] and
    .approvalSubject.currentTaskEvent.authorRole=="USER" and
    (.approvalSubject.currentTaskEvent.messageDigest|test("^[a-f0-9]{64}$"))
  ' "$approval" >/dev/null || reject APPROVAL_SHAPE_INVALID approval
  [ "$(jq -cS '.approvalSubject' "$approval" | sha_stream)" = "$(jq -r '.approvalSubjectDigest' "$approval")" ] || reject APPROVAL_SUBJECT_DIGEST_MISMATCH approval
  local mode
  mode=$(jq -r '.approvalSubject.currentTaskEvent.mode' "$approval")
  if [ "$mode" = TEST_ONLY ]; then [ "$SELF_TEST" -eq 1 ] || reject TEST_ONLY_FORBIDDEN approval; else [ "$mode" = NORMAL ] || reject APPROVAL_EVENT_INVALID mode; fi

  local expected_roles actual_roles
  expected_roles=$(jq -cS '.approvalArtifactRoles|sort' "$manifest")
  actual_roles=$(jq -cS '.approvalSubject.artifacts|map(.role)|sort' "$approval")
  [ "$actual_roles" = "$expected_roles" ] || reject APPROVAL_ARTIFACT_SET_MISMATCH roles
  local count index role path expected resolved actual
  count=$(jq '.approvalSubject.artifacts|length' "$approval"); index=0
  while [ "$index" -lt "$count" ]; do
    role=$(jq -r ".approvalSubject.artifacts[$index].role" "$approval")
    path=$(jq -r ".approvalSubject.artifacts[$index].path" "$approval")
    expected=$(jq -r ".approvalSubject.artifacts[$index].rawFileSha256" "$approval")
    resolved=$(repo_path "$path"); require_file "$resolved"; actual=$(sha_file "$resolved")
    [ "$actual" = "$expected" ] || reject "STALE_$role" "$path"
    index=$((index+1))
  done
  [ "$(jq -r '.approvalSubject.artifacts[]|select(.role=="BASE_PLAN")|.rawFileSha256' "$approval")" = "$BASE_PLAN_SHA" ] || reject STALE_BASE_PLAN approval
  [ "$(jq -r '.approvalSubject.artifacts[]|select(.role=="P10_LITE_AMENDMENT")|.rawFileSha256' "$approval")" = "$(sha_file "$amendment")" ] || reject STALE_AMENDMENT approval
  [ "$(jq -r '.approvalSubject.artifacts[]|select(.role=="P10_LITE_MANIFEST")|.rawFileSha256' "$approval")" = "$(sha_file "$manifest")" ] || reject STALE_LITE_MANIFEST approval
  [ "$(jq -r '.approvalSubject.artifacts[]|select(.role=="P10_LITE_CONTROLLER")|.rawFileSha256' "$approval")" = "$(sha_file "$0")" ] || reject STALE_LITE_CONTROLLER approval

  jq -cnS --arg amendmentDigest "$(sha_file "$amendment")" '{controllerVerdict:"CONTROL_ACCEPT",effectiveEligibleNextEdges:["A10->B01"],amendmentDigest:$amendmentDigest,runtimeContractsAccepted:false}'
}

make_approval() {
  local manifest=$1 output=$2 mode=$3
  jq -nS --arg mode "$mode" --argjson artifacts "$(jq -c '[.basePlan,.amendment,{role:"P10_LITE_MANIFEST",path:"docs/superpowers/plans/codeclew-p10-lite-manifest-v1.json"},.controller,.rejectedP10Evidence,.frozenRuntimePrototype]' "$manifest")" '
    {schemaVersion:"codeclew-p10-lite-approval/1",approvalSubject:{artifacts:($artifacts|map(.rawFileSha256 = (.rawFileSha256 // ""))),currentTaskEvent:{mode:$mode,taskId:"goal-task",messageId:"approval-message",authorRole:"USER",messageDigest:("a"*64)}},approvalSubjectDigest:"",humanDecision:"HUMAN_APPROVED",createdAt:"2026-08-09T00:00:00Z",canonicalScope:"JQ_1_7_SORTED_COMPACT_INTEGER_JSON"}
  ' > "$output"
  local index path digest count
  count=$(jq '.approvalSubject.artifacts|length' "$output"); index=0
  while [ "$index" -lt "$count" ]; do
    path=$(jq -r ".approvalSubject.artifacts[$index].path" "$output"); digest=$(sha_file "$ROOT/$path")
    jq --argjson index "$index" --arg digest "$digest" '.approvalSubject.artifacts[$index].rawFileSha256=$digest' "$output" > "$output.tmp" && mv "$output.tmp" "$output"
    index=$((index+1))
  done
  digest=$(jq -cS '.approvalSubject' "$output" | sha_stream)
  jq --arg digest "$digest" '.approvalSubjectDigest=$digest' "$output" > "$output.tmp" && mv "$output.tmp" "$output"
}

self_test() {
  SELF_TEST=1
  local plan="$ROOT/docs/superpowers/plans/2026-08-09-codeclew-optimized-research-foundation-plan.md"
  local amendment="$ROOT/docs/superpowers/plans/2026-08-09-codeclew-p10-lite-amendment.md"
  local manifest="$ROOT/docs/superpowers/plans/codeclew-p10-lite-manifest-v1.json"
  local temp approval out status passed=0 total=0
  temp=$(mktemp -d); approval="$temp/approval.json"; make_approval "$manifest" "$approval" TEST_ONLY
  total=$((total+1)); out=$(verify_package "$plan" "$amendment" "$manifest" "$approval") && [ "$(jq -r '.controllerVerdict' <<<"$out")" = CONTROL_ACCEPT ] && passed=$((passed+1))
  negative() {
    local name=$1 expected=$2 filter=$3 rc output digest
    local a="$temp/$name.json"
    jq "$filter" "$approval" > "$a"
    digest=$(jq -cS '.approvalSubject' "$a" | sha_stream)
    jq --arg digest "$digest" '.approvalSubjectDigest=$digest' "$a" > "$a.tmp" && mv "$a.tmp" "$a"
    total=$((total+1)); set +e; output=$(verify_package "$plan" "$amendment" "$manifest" "$a"); rc=$?; set -e
    [ "$rc" -eq 2 ] && [ "$(jq -r '.rejectCode' <<<"$output")" = "$expected" ] && passed=$((passed+1)) || { printf '%s expected %s got %s\n' "$name" "$expected" "$output" >&2; return 1; }
  }
  negative stale-plan STALE_BASE_PLAN '(.approvalSubject.artifacts[]|select(.role=="BASE_PLAN")|.rawFileSha256)=("b"*64)'
  negative stale-amendment STALE_P10_LITE_AMENDMENT '(.approvalSubject.artifacts[]|select(.role=="P10_LITE_AMENDMENT")|.rawFileSha256)=("b"*64)'
  negative stale-manifest STALE_P10_LITE_MANIFEST '(.approvalSubject.artifacts[]|select(.role=="P10_LITE_MANIFEST")|.rawFileSha256)=("b"*64)'
  negative bad-controller STALE_P10_LITE_CONTROLLER '(.approvalSubject.artifacts[]|select(.role=="P10_LITE_CONTROLLER")|.rawFileSha256)=("b"*64)|.approvalSubjectDigest=""'
  negative stale-report STALE_REJECTED_P10_EVIDENCE '(.approvalSubject.artifacts[]|select(.role=="REJECTED_P10_EVIDENCE")|.rawFileSha256)=("b"*64)'
  negative stale-prototype STALE_FROZEN_RUNTIME_PROTOTYPE '(.approvalSubject.artifacts[]|select(.role=="FROZEN_RUNTIME_PROTOTYPE")|.rawFileSha256)=("b"*64)'
  negative bad-set APPROVAL_ARTIFACT_SET_MISMATCH '.approvalSubject.artifacts |= map(select(.role!="REJECTED_P10_EVIDENCE"))|.approvalSubjectDigest=""'
  negative invalid-event APPROVAL_SHAPE_INVALID '.approvalSubject.currentTaskEvent.messageDigest="not-a-digest"'
  negative extra-field APPROVAL_SHAPE_INVALID '.unexpected=true'
  SELF_TEST=0; negative external-test-only TEST_ONLY_FORBIDDEN '.'; SELF_TEST=1
  jq -cnS --argjson total "$total" --argjson passed "$passed" '{schemaVersion:"codeclew-p10-lite-self-test/1",status:(if $total==$passed then "PASS" else "FAIL" end),total:$total,passed:$passed,runtimeContractsAccepted:false}'
  rm -rf "$temp"; [ "$total" -eq "$passed" ]
}

if [ "$#" -eq 1 ] && [ "$1" = --self-test ]; then set -e; self_test; exit $?; fi
[ "$#" -eq 8 ] || usage
PLAN= AMENDMENT= MANIFEST= APPROVAL=
while [ "$#" -gt 0 ]; do
  case "$1" in --plan) PLAN=$2;; --amendment) AMENDMENT=$2;; --manifest) MANIFEST=$2;; --approval) APPROVAL=$2;; *) usage;; esac
  shift 2
done
verify_package "$PLAN" "$AMENDMENT" "$MANIFEST" "$APPROVAL"
