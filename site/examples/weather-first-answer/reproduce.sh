#!/usr/bin/env bash
# A linear replay of the documented public SOURCE example, not application execution.
set -eu
example_dir=${1:-"$PWD/clew-first-answer"}
if [ -e "$example_dir" ]; then
  echo "Choose a new directory: $example_dir already exists" >&2
  exit 1
fi
mkdir -p "$example_dir"
example_dir=$(cd "$example_dir" && pwd)
clew --version

git clone --quiet https://github.com/codeclew/clew-weather-demo "$example_dir/weather"
git -C "$example_dir/weather" checkout --quiet --detach f0e5c376a2333ec6b86b114039fe10921b6f3c4a
clew docs init --root "$example_dir/answer" --title 'Weather first answer' > "$example_dir/init.json"
cat > "$example_dir/dashboard.json" <<'JSON'
{
  "schema": "codeclew-documentation-service/1.0",
  "id": "dashboard",
  "title": "Dashboard save timeout",
  "repositoryId": "clew-weather-demo",
  "repository": "https://github.com/codeclew/clew-weather-demo",
  "language": "typescript",
  "profile": "source-syntax",
  "targetRef": "f0e5c376a2333ec6b86b114039fe10921b6f3c4a",
  "source": {"roots": ["services/dashboard/src/main.ts"], "dialect": "7.0.2"}
}
JSON
clew docs service list --root "$example_dir/answer" > "$example_dir/catalogue.json"
input_digest=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["inputDigest"])' "$example_dir/catalogue.json")
clew docs service add --root "$example_dir/answer" --input "$example_dir/dashboard.json" --expected-input-digest "$input_digest" > "$example_dir/service.json"
clew docs bind --root "$example_dir/answer" --service dashboard --repo "$example_dir/weather" > "$example_dir/bind.json"
# A first Check has no authored baseline; v0.13.34 returns exit 3 for that state.
check_exit=0
clew docs check --root "$example_dir/answer" --service dashboard > "$example_dir/check.json" || check_exit=$?
snapshot=$(python3 - "$example_dir/check.json" "$check_exit" <<'PY'
import json,sys
check=json.load(open(sys.argv[1]))
assert check.get('status')=='CHECKED' and not check.get('unresolved'),check
assert check['services']['dashboard']['revision']=='f0e5c376a2333ec6b86b114039fe10921b6f3c4a',check
assert check['services']['dashboard']['coverage']=='SYNTAX',check
if int(sys.argv[2]):
    assert int(sys.argv[2])==3 and check['freshness']['reasons']==['MISSING_BASELINE'],check
print(check['snapshot'])
PY
)
clew docs context --root "$example_dir/answer" --snapshot "$snapshot" --service dashboard --symbol 'source:services/dashboard/src/main.ts/package:/savePlace/7a5f9ccc269b81055666' --format compact --limit 100 > "$example_dir/context.json"
clew docs context --root "$example_dir/answer" --snapshot "$snapshot" --service dashboard --source dashboard-7970ac3e2a8cf3f45b4a --format raw > "$example_dir/source.json"
cat > "$example_dir/selection.json" <<'JSON'
[{"id":"weather-save-timeout","service":"dashboard","endpointDeclaration":"dashboard:symbol:7970ac3e2a8cf3f45b4a6217","workerDeclaration":"dashboard:symbol:7970ac3e2a8cf3f45b4a6217"}]
JSON
clew docs pages render --root "$example_dir/answer" --snapshot "$snapshot" --input "$example_dir/selection.json" --output "$example_dir/pages" > "$example_dir/render.json"
printf 'Saved question context: %s/context.json\nNative result: %s/pages/weather-save-timeout-worker.html\nSnapshot: %s\n' "$example_dir" "$example_dir" "$snapshot"
