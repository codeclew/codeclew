//! Multi-compilation and incremental-fact membership regressions (2026-09-16
//! multisource-incremental-facts-v2). One documented service may select several
//! explicit same-repository compilation scopes (e.g. a web module delegating
//! through sibling flow/common modules). Compilation selection is a plural
//! authored set; empty and duplicated declarations are typed validation errors.

use clew::documentation::access::RequestScope;
use clew::documentation::analysis;
use clew::documentation::cache;
use clew::documentation::fact_index;
use clew::documentation::model::{Service, ServiceEvidence};
use clew::documentation::store::{Repository, validate_service};
use clew::error::ErrorCode;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;

fn service(compilation: &str, compilations: Vec<&str>) -> Service {
    Service {
        schema: "codeclew-documentation-service/1.0".into(),
        id: "orders".into(),
        title: "Orders".into(),
        repository_id: "orders".into(),
        repository: "https://example.invalid/orders".into(),
        language: "java".into(),
        profile: "java-17plus-maven-read-only".into(),
        compilations: if compilations.is_empty() {
            vec![compilation.to_owned()]
        } else {
            compilations.into_iter().map(str::to_owned).collect()
        },
        source: None,
        modules: None,
        target_ref: "main".into(),
        source_link_template: None,
        contract_files: vec![],
        annotation_processor_paths: vec![],
    }
}

/// One service selects web, flow and common compilation scopes without
/// inventing separate services: the plural set is validated and normalizes to
/// exactly the authored scopes.
#[test]
fn one_service_selects_multiple_compilation_scopes() {
    let s = service("", vec![":web:/main", ":flow:/main", ":common:/main"]);
    validate_service(&s).unwrap();
    assert_eq!(
        s.effective_compilations(),
        vec![":web:/main", ":flow:/main", ":common:/main"],
        "a plural selection must be honored verbatim and must not invent services"
    );
}

/// Duplicated scopes cannot multiply work and empty selections are rejected,
/// while the order of the plural set has no semantic effect on the normalized
/// result (no reordering is performed, so the authored order is preserved).
#[test]
fn duplicate_and_empty_selections_are_rejected() {
    let duplicated = service("", vec![":/main", ":/main"]);
    let error = validate_service(&duplicated).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);

    let empty_selector = service("", vec![":/main", ""]);
    let error = validate_service(&empty_selector).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);

    let no_selection = service("", vec![]);
    let error = validate_service(&no_selection).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
}

/// An oversized plural selection (more than 128 scopes) is rejected so a
/// single service cannot claim an unbounded compilation authority.
#[test]
fn compilation_limit_rejects_129_doc_service_selectors() {
    // 128 distinct synthetic selectors are admitted; the 129th is rejected so
    // the count guard, not selector duplication, is what bounds selection.
    let admitted = (0..128)
        .map(|i| format!(":/module{i}:main"))
        .collect::<Vec<_>>();
    let mut s = Service {
        compilations: admitted,
        ..service("", vec![])
    };
    validate_service(&s).unwrap();
    assert_eq!(s.compilations.len(), 128);

    s.compilations = (0..129)
        .map(|i| format!(":/module{i}:main"))
        .collect::<Vec<_>>();
    let error = validate_service(&s).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert!(error.message.contains("at most 128"));
}

// ---------------------------------------------------------------------------
// Step 3: scope-aware capture and resolution.
// ---------------------------------------------------------------------------

fn jvm_service() -> Service {
    service("", vec![":/web:main", ":/flow:main", ":/common:main"])
}

/// A synthetic DECLARATION fact for `symbol` at `file`, optionally carrying a
/// compilation scope and a CALL event to `event_target`.
fn declaration(symbol: &str, file: &str, scope: Option<&str>, event_target: Option<&str>) -> Value {
    let mut fact = json!({
        "kind": "DECLARATION",
        "symbolIdentity": symbol,
        "ownerIdentity": symbol.split('.').next().unwrap_or(""),
        "name": symbol.split('.').next_back().unwrap_or(""),
        "file": file,
        "startLine": 1,
        "endLine": 1,
        "resolution": "COMPILER_EXACT",
        "documentation": {
            "events": if let Some(target) = event_target {
                json!([{"kind":"CALL","target":format!("method:{target}"),"resolution":"COMPILER_EXACT"}])
            } else {
                json!([])
            }
        }
    });
    if let Some(scope) = scope {
        fact["scope"] = json!({"compilation":scope});
    }
    fact
}

fn project_facts(facts: Vec<(Value, String)>) -> ServiceEvidence {
    let files = BTreeMap::from([
        ("web/Web.java".to_string(), "class Web {}".to_string()),
        ("flow/Flow.java".to_string(), "class Flow {}".to_string()),
        (
            "common/Common.java".to_string(),
            "class Common {}".to_string(),
        ),
    ]);
    analysis::project(
        &jvm_service(),
        &"a".repeat(40),
        "service-digest",
        "DEVELOPMENT",
        "PARTIAL",
        facts,
        &files,
        false,
    )
    .unwrap()
}

fn symbol_observations<'a>(
    e: &'a ServiceEvidence,
    symbol: &str,
) -> Vec<&'a clew::documentation::model::Observation> {
    e.observations
        .values()
        .filter(|o| o.kind == "SYMBOL" && o.symbol == symbol)
        .collect()
}

/// The same symbol admitted under two compilation scopes with incompatible
/// candidate bodies is retained as two scope-distinct observations (never
/// last-write-wins overwritten) and flagged as a visible SCOPE_AMBIGUOUS
/// boundary.
#[test]
fn scope_conflict_is_retained_and_flagged_not_overwritten() {
    let facts = vec![
        (
            declaration(
                "example.Common",
                "common/Common.java",
                Some(":/web:main"),
                Some("x"),
            ),
            "b1".into(),
        ),
        (
            declaration(
                "example.Common",
                "common/Common.java",
                Some(":/common:main"),
                Some("y"),
            ),
            "b2".into(),
        ),
    ];
    let evidence = project_facts(facts);
    let matching = symbol_observations(&evidence, "example.Common");
    assert_eq!(
        matching.len(),
        2,
        "conflicting scope candidates must both be retained, not overwritten"
    );
    let scopes: Vec<&str> = matching
        .iter()
        .map(|o| o.normalized["scope"].as_str().unwrap_or(""))
        .collect();
    assert!(scopes.contains(&":/web:main") && scopes.contains(&":/common:main"));
    assert!(
        evidence
            .boundaries
            .iter()
            .any(|b| b == "SCOPE_AMBIGUOUS:example.Common"),
        "an incompatible multi-scope symbol must be an explicit boundary: {:?}",
        evidence.boundaries
    );
}

/// Identical symbol payloads across scopes are not ambiguous: both scope
/// observations coexist under distinct identities with no SCOPE_AMBIGUOUS
/// boundary.
#[test]
fn identical_symbol_across_scopes_is_not_ambiguous() {
    let facts = vec![
        (
            declaration(
                "example.Common",
                "common/Common.java",
                Some(":/web:main"),
                Some("x"),
            ),
            "b1".into(),
        ),
        (
            declaration(
                "example.Common",
                "common/Common.java",
                Some(":/common:main"),
                Some("x"),
            ),
            "b2".into(),
        ),
    ];
    let evidence = project_facts(facts);
    assert_eq!(symbol_observations(&evidence, "example.Common").len(), 2);
    assert!(
        !evidence
            .boundaries
            .iter()
            .any(|b| b.starts_with("SCOPE_AMBIGUOUS:")),
        "identical candidates across scopes must not be ambiguous: {:?}",
        evidence.boundaries
    );
}

/// A web -> flow -> common call chain admitted through the compiler's exact
/// resolution surfaces as cross-scope observations: the web declaration's CALL
/// event targets the flow body, which itself is present as an admitted
/// observation. The chain stays within the admitted bodies rather than
/// inventing external or dynamic dispatch.
#[test]
fn cross_scope_call_chain_surfaces_admitted_bodies() {
    let facts = vec![
        (
            declaration(
                "example.WebController",
                "web/Web.java",
                Some(":/web:main"),
                Some("example.InventoryClient#reserve"),
            ),
            "b1".into(),
        ),
        (
            declaration(
                "example.InventoryClient",
                "flow/Flow.java",
                Some(":/flow:main"),
                Some("example.CommonUtil#dedupe"),
            ),
            "b2".into(),
        ),
        (
            declaration(
                "example.CommonUtil",
                "common/Common.java",
                Some(":/common:main"),
                None,
            ),
            "b3".into(),
        ),
    ];
    let evidence = project_facts(facts);
    assert_eq!(
        symbol_observations(&evidence, "example.WebController").len(),
        1
    );
    assert_eq!(
        symbol_observations(&evidence, "example.InventoryClient").len(),
        1
    );
    assert_eq!(
        symbol_observations(&evidence, "example.CommonUtil").len(),
        1
    );
    // The web body's CALL event references the flow target by exact identity.
    let web = symbol_observations(&evidence, "example.WebController")[0];
    let target = web.normalized["documentation"]["events"][0]["target"]
        .as_str()
        .expect("web body must expose its exact call target");
    assert_eq!(target, "method:example.InventoryClient#reserve");
    assert_eq!(
        web.normalized["scope"], ":/web:main",
        "cross-scope calls must carry the admitting scope"
    );
}

// ---------------------------------------------------------------------------
// Step 4: RestClient fluent-chain egress through a real JDK 21 compiler.
// ---------------------------------------------------------------------------

/// Minimal local Spring stubs so the analyzer resolves `RestClient` fluent
/// chains without the real dependency or any network.
const RESTCLIENT_STUBS: &[(&str, &str)] = &[
    (
        "org/springframework/http/HttpMethod.java",
        "package org.springframework.http; public enum HttpMethod { GET, HEAD, POST, PUT, PATCH, DELETE, OPTIONS }",
    ),
    (
        "org/springframework/beans/factory/annotation/Value.java",
        "package org.springframework.beans.factory.annotation; public @interface Value { String value(); }",
    ),
    (
        "org/springframework/web/client/RestTemplate.java",
        r#"
package org.springframework.web.client;
public class RestTemplate {
    public <T> T postForObject(String url, Object request, Class<T> type) { return null; }
    public <T> T getForObject(String url, Class<T> type) { return null; }
}
"#,
    ),
    (
        "org/springframework/web/client/RestClient.java",
        r#"
package org.springframework.web.client;
import org.springframework.http.HttpMethod;
public class RestClient {
    public static RestClient create() { return new RestClient(); }
    public RequestBodySpec get() { return null; }
    public RequestBodySpec head() { return null; }
    public RequestBodySpec post() { return null; }
    public RequestBodySpec put() { return null; }
    public RequestBodySpec patch() { return null; }
    public RequestBodySpec delete() { return null; }
    public RequestBodySpec options() { return null; }
    public RequestBodySpec method(HttpMethod m) { return null; }
    public interface RequestBodySpec {
        RequestBodySpec uri(String uri, Object... uriVariables);
        ResponseSpec retrieve();
    }
    public interface ResponseSpec { <T> T body(Class<T> type); }
}
"#,
    ),
];

/// Run the embedded JDK analyzer on `sources` and return the parsed DECLARATION
/// facts as JSON values (a real compiler pass, not a mock).
fn run_jdk_analyzer(sources: &[(&str, &str)]) -> Vec<Value> {
    let java_home = std::env::var_os("JAVA_HOME")
        .unwrap_or_else(|| panic!("JAVA_HOME must be set to run the JDK analyzer"));
    let java = std::path::PathBuf::from(&java_home).join("bin/java");
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repository");
    let mut manifest_paths = Vec::new();
    for (package_or_file, body) in RESTCLIENT_STUBS.iter().chain(sources.iter()) {
        let file = root.join(package_or_file);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, body).unwrap();
        manifest_paths.push(package_or_file.to_string());
    }
    let analyzer = temp.path().join("CodeclewJavaAnalyzer.java");
    fs::write(&analyzer, clew::java_adapter_v2::JAVA_ANALYZER_SOURCE).unwrap();
    let manifest = temp.path().join("sources.txt");
    fs::write(&manifest, manifest_paths.join("\n")).unwrap();
    let classpath = temp.path().join("classpath.txt");
    fs::write(&classpath, "").unwrap();
    let output = std::process::Command::new(java)
        .arg("--source")
        .arg("17")
        .arg(&analyzer)
        .arg(&root)
        .arg(&manifest)
        .arg(&classpath)
        .arg("21")
        .arg("")
        .arg("")
        .arg("")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "JDK analyzer failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Egress observations carried by a DECLARATION fact's documentation events.
fn egress(observation: &Value) -> Vec<&Value> {
    observation["documentation"]["events"]
        .as_array()
        .map(|events| {
            events
                .iter()
                .filter(|e| e["http"]["method"].is_string())
                .collect()
        })
        .unwrap_or_default()
}

/// A real JDK 21 compiler-to-docs pass proves the repaired fluent-chain
/// traversal: `client.post().uri("/orders").retrieve().body(...)` yields
/// exactly one usable egress (POST, /orders), configured-but-unexecuted
/// request specifications yield none, absolute URLs split authority from path,
/// and `method(HttpMethod.POST)` resolves the verb statically.
#[test]
fn restclient_fluent_chains_produce_single_usable_egress_on_jdk21() {
    let fixture = (
        "example/OrdersClient.java",
        r#"
package example;
import org.springframework.web.client.RestClient;
import org.springframework.web.client.RestTemplate;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.http.HttpMethod;
public class OrdersClient {
    private final RestClient http = RestClient.create();
    private final RestTemplate rt = new RestTemplate();
    @Value("${orders.base-url}")
    private String base;
    public String reserve(String id) {
        return http.post().uri("/orders/{id}", id).retrieve().body(String.class);
    }
    public String list() {
        return http.get().uri("https://api.example.com/v1/items").retrieve().body(String.class);
    }
    public void configure() {
        http.post().retrieve();
    }
    public String viaMethod(String id) {
        return http.method(HttpMethod.POST).uri("/method/{id}", id).retrieve().body(String.class);
    }
    public String absolute() {
        return http.options().uri("http://b.example/").retrieve().body(String.class);
    }
    public String legacy(String body) {
        return rt.postForObject(base + "/legacy", body, String.class);
    }
    public void lookalike(Impostor impostor) {
        impostor.post().uri("/fake").retrieve();
    }
}
class Impostor {
    public Impostor post() { return this; }
    public Impostor uri(String u) { return this; }
    public Impostor retrieve() { return this; }
}
"#,
    );
    let facts = run_jdk_analyzer(&[fixture]);
    let declarations: Vec<Value> = facts
        .into_iter()
        .filter(|f| f["kind"] == "DECLARATION")
        .collect();
    assert!(!declarations.is_empty());

    // Full compiler-to-docs projection: each method's CALL egress events must
    // be exactly one per executed request, correctly attributed.
    let evidence = project_facts(
        declarations
            .into_iter()
            .map(|v| (v, "binding".into()))
            .collect(),
    );
    let egress_by_symbol: Vec<(String, Vec<&Value>)> = evidence
        .observations
        .values()
        .filter(|o| o.kind == "SYMBOL" && o.normalized["name"].is_string())
        .map(|o| {
            (
                o.normalized["name"].as_str().unwrap().to_string(),
                egress(&o.normalized),
            )
        })
        .collect();

    let find = |name: &str| {
        egress_by_symbol
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("missing declaration {name}"))
            .1
            .clone()
    };

    // A standard fluent chain produces exactly one usable egress with the
    // template path kept as a template.
    let reserve = find("reserve");
    assert_eq!(reserve.len(), 1, "one egress per executed request");
    assert_eq!(reserve[0]["http"]["method"], "POST");
    assert_eq!(reserve[0]["http"]["path"], "/orders/{id}");
    assert_eq!(reserve[0]["http"]["adapter"], "SPRING_REST_CLIENT_URI/1.0");

    // An absolute literal URL is split into authority and normalized path.
    let list = find("list");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["http"]["method"], "GET");
    assert_eq!(list[0]["http"]["authority"], "api.example.com");
    assert_eq!(list[0]["http"]["path"], "/v1/items");

    // A configured-but-unexecuted request specification is not an egress.
    assert!(find("configure").is_empty());

    // method(HttpMethod.CONSTANT) resolves the verb statically.
    let via = find("viaMethod");
    assert_eq!(via.len(), 1);
    assert_eq!(via[0]["http"]["method"], "POST");
    assert_eq!(via[0]["http"]["path"], "/method/{id}");

    // An absolute URL without a path keeps a bare root path.
    let absolute = find("absolute");
    assert_eq!(absolute.len(), 1);
    assert_eq!(absolute[0]["http"]["authority"], "b.example");
    assert_eq!(absolute[0]["http"]["path"], "/");

    // RestTemplate literal-suffix extraction still works and preserves the
    // declared destination configuration key.
    let legacy = find("legacy");
    assert_eq!(legacy.len(), 1);
    assert_eq!(
        legacy[0]["http"]["adapter"],
        "SPRING_REST_TEMPLATE_LITERAL_SUFFIX/1.0"
    );
    assert_eq!(legacy[0]["http"]["method"], "POST");
    assert_eq!(legacy[0]["http"]["path"], "/legacy");
    assert_eq!(legacy[0]["http"]["destinationConfigKey"], "orders.base-url");

    // An unrelated fluent lookalike named post/uri/retrieve must not be
    // confused with a Spring RestClient egress.
    assert!(find("lookalike").is_empty());
}

// ---------------------------------------------------------------------------
// Step 8: deterministic storage-scale evidence over the membership index.
// ---------------------------------------------------------------------------

/// A synthetic web/flow/common reactor with >=1,300 distinct generated source
/// records and tens of thousands of fact memberships proves the indexed store
/// deterministically: payloads are stored once, a point lookup reads one page,
/// a single-fact delta rewrites only the affected page, and logical removal is
/// observable without scanning every payload. This measures storage behavior;
/// compiler behavior is measured by the JDK21 RestClient test above.
#[test]
fn storage_scale_unique_io_and_bounded_deltas_are_deterministic() {
    let temp = tempfile::tempdir().unwrap();
    Repository::init(temp.path(), "Architecture").unwrap();
    let repo = Repository::open(temp.path()).unwrap();

    // >=1,300 distinct source payloads.
    const DISTINCT_PAYLOADS: usize = 1300;
    let mut source_refs = Vec::with_capacity(DISTINCT_PAYLOADS);
    for i in 0..DISTINCT_PAYLOADS {
        let ref_ = cache::put(
            &repo,
            "codeclew-documentation-sources/1.0",
            format!("source record {i} with stable body").as_bytes(),
        )
        .unwrap();
        source_refs.push(ref_);
    }

    // Tens of thousands of fact memberships across web/flow/common scopes,
    // each bound to one of the distinct source payloads (shared across scopes).
    const MEMBERSHIPS: usize = 20_000;
    const SCOPES: [&str; 3] = [":/web:main", ":/flow:main", ":/common:main"];
    let memberships: Vec<fact_index::FactOccurrence> = (0..MEMBERSHIPS)
        .map(|i| fact_index::FactOccurrence {
            key: fact_index::OccurrenceKey {
                repository: "orders".into(),
                revision: "a".repeat(40),
                source_state: "EXACT_SNAPSHOT_TEXT".into(),
                scope: SCOPES[i % 3].into(),
                domain: "analysis:java-compiler-facts".into(),
                semantic: format!("example/Module{}/Fact{}", i % 3, i),
            },
            payload: source_refs[i % DISTINCT_PAYLOADS].clone(),
            kind: "SYMBOL".into(),
            file: format!(
                "src/{}/F{}.java",
                SCOPES[i % 3]
                    .trim_start_matches(':')
                    .split('/')
                    .next()
                    .unwrap_or("x"),
                i
            ),
            symbol: format!("Fact{i}"),
        })
        .collect();

    // Apply the full closure in one batched delta: each bucket page is written
    // once, and every distinct payload is persisted exactly once.
    let root = fact_index::empty_root();
    let root = fact_index::apply_delta(&repo, &root, &memberships, &[]).unwrap();
    fact_index::verify_root(&repo, &root).unwrap();

    // Unique payload storage: all 1300 distinct sources are referenced and each
    // is stored once, so a full-reachability report finds zero unreferenced
    // payloads and the referenced set equals the distinct payload count.
    let report = fact_index::reclaimable(&repo, &root).unwrap();
    assert_eq!(
        report.referenced_payloads, DISTINCT_PAYLOADS,
        "the index must reference exactly the distinct source payloads: {report:?}"
    );
    assert_eq!(
        report.unreferenced_payloads, 0,
        "with every payload referenced, nothing is logically reclaimable: {report:?}"
    );

    // Request-scoped hydration reads each unique required payload once.
    let mut scope = RequestScope::new(&repo);
    let sample: Vec<cache::ObjectRef> = (0..100).map(|i| source_refs[i % 50].clone()).collect();
    let hydrated = scope.hydrate(&sample).unwrap();
    assert_eq!(
        hydrated.len(),
        50,
        "100 overlapping refs collapse to 50 unique payloads"
    );
    assert_eq!(scope.counters().unique_payload_fetches, 50);

    // A single-fact replacement/removal rewrites only the affected bucket page
    // and never scans every payload.
    let before: Vec<Option<String>> = root
        .buckets
        .iter()
        .map(|r| r.as_ref().map(|r| r.digest.clone()))
        .collect();
    let target_key = memberships[9999].key.clone();
    let after_root =
        fact_index::apply_delta(&repo, &root, &[], std::slice::from_ref(&target_key)).unwrap();
    let after: Vec<Option<String>> = after_root
        .buckets
        .iter()
        .map(|r| r.as_ref().map(|r| r.digest.clone()))
        .collect();
    let changed = before
        .iter()
        .zip(after.iter())
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(
        changed, 1,
        "a single-fact removal must rewrite exactly one bucket page, not every payload"
    );
    assert!(
        fact_index::lookup(&repo, &after_root, &target_key)
            .unwrap()
            .is_none()
    );

    // Removing the whole web scope logically detaches its memberships; the
    // shared source payloads become observable as reclaimable without deletion.
    let after_scope =
        fact_index::replace_scope(&repo, &after_root, ":/web:main", &[], true).unwrap();
    let scope_report = fact_index::reclaimable(&repo, &after_scope).unwrap();
    assert!(
        scope_report.unreferenced_payloads > 0,
        "removing a shared scope must surface its now-unreferenced payloads: {scope_report:?}"
    );
    // Physical objects remain (no destructive cleanup in this run).
    assert_eq!(
        cache::owned_digests(&repo, 8192).unwrap().len(),
        scope_report.owned_payloads
    );
}
