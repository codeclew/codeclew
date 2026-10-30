//! Bounded declaration reader, not a PostgreSQL execution or migration engine.
use serde_json::{Value, json};

#[derive(Clone, Debug)]
struct Token {
    a: usize,
    b: usize,
    quoted: bool,
}
impl Token {
    fn raw<'a>(&self, s: &'a str) -> &'a str {
        &s[self.a..self.b]
    }
    fn is(&self, s: &str, word: &str) -> bool {
        !self.quoted && self.raw(s).eq_ignore_ascii_case(word)
    }
    fn identifier(&self, s: &str) -> bool {
        self.raw(s).starts_with('"')
            || self
                .raw(s)
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
                && !self.quoted
    }
    fn key(&self, s: &str) -> String {
        let r = self.raw(s);
        if r.starts_with('"') {
            r[1..r.len() - 1].replace("\"\"", "\"")
        } else {
            r.to_ascii_lowercase()
        }
    }
}
fn range(a: usize, b: usize) -> Value {
    json!({"coordinateDomain":"SOURCE_RECORD_UTF8_BYTES","byteStart":a,"byteEnd":b})
}
fn gap(out: &mut Vec<Value>, code: &str, a: usize, b: usize) {
    out.push(json!({"code":code,"source":range(a,b)}));
}

fn lex(s: &str) -> (Vec<Token>, Vec<Value>) {
    let b = s.as_bytes();
    let mut i = 0;
    let mut tokens = Vec::new();
    let mut gaps = Vec::new();
    while i < b.len() {
        if i == 0 && s.starts_with('\u{feff}') {
            i = 3;
            continue;
        }
        if tokens.len() >= 32768 {
            gap(&mut gaps, "TOKEN_BOUND", i, b.len());
            break;
        }
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b[i..].starts_with(b"--") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b[i..].starts_with(b"/*") {
            let a = i;
            i += 2;
            let mut depth = 1;
            while i < b.len() && depth > 0 {
                if b[i..].starts_with(b"/*") {
                    depth += 1;
                    i += 2;
                } else if b[i..].starts_with(b"*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
                if depth > 32 {
                    gap(&mut gaps, "COMMENT_DEPTH_BOUND", a, b.len());
                    return (tokens, gaps);
                }
            }
            if depth > 0 {
                gap(&mut gaps, "UNCLOSED_COMMENT", a, b.len());
                break;
            }
            continue;
        }
        let a = i;
        if matches!(b[i], b'\'' | b'"') {
            let quote = b[i];
            let escaped = quote == b'\''
                && tokens
                    .last()
                    .is_some_and(|t: &Token| t.b == i && t.is(s, "E"));
            i += 1;
            let mut closed = false;
            while i < b.len() {
                if escaped && b[i] == b'\\' {
                    i = (i + 2).min(b.len());
                    continue;
                }
                if b[i] == quote {
                    i += 1;
                    if i < b.len() && b[i] == quote {
                        i += 1;
                    } else {
                        closed = true;
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            if !closed {
                gap(&mut gaps, "UNCLOSED_QUOTE", a, b.len());
                break;
            }
            tokens.push(Token {
                a,
                b: i,
                quoted: true,
            });
            continue;
        }
        if b[i] == b'$' {
            let mut z = i + 1;
            while z < b.len() && (b[z].is_ascii_alphanumeric() || b[z] == b'_') {
                z += 1;
            }
            if z < b.len() && b[z] == b'$' {
                let delimiter = &s[i..=z];
                i = z + 1;
                if let Some(end) = s[i..].find(delimiter) {
                    i += end + delimiter.len();
                    tokens.push(Token {
                        a,
                        b: i,
                        quoted: true,
                    });
                    continue;
                }
                gap(&mut gaps, "UNCLOSED_DOLLAR_QUOTE", a, b.len());
                break;
            }
        }
        let c = s[i..].chars().next().unwrap();
        if c.is_alphanumeric() || c == '_' {
            i += c.len_utf8();
            while i < b.len() {
                let c = s[i..].chars().next().unwrap();
                if c.is_alphanumeric() || matches!(c, '_' | '$') {
                    i += c.len_utf8();
                } else {
                    break;
                }
            }
        } else {
            i += c.len_utf8();
        }
        tokens.push(Token {
            a,
            b: i,
            quoted: false,
        });
    }
    (tokens, gaps)
}
fn parts(t: &[Token], s: &str) -> Option<Vec<(usize, usize)>> {
    let mut result = Vec::new();
    let (mut from, mut depth) = (0, 0usize);
    for (i, x) in t.iter().enumerate() {
        if x.is(s, "(") || x.is(s, "[") {
            depth += 1;
            if depth > 32 {
                return None;
            }
        }
        if x.is(s, ")") || x.is(s, "]") {
            depth = depth.checked_sub(1)?;
        }
        if x.is(s, ",") && depth == 0 {
            result.push((from, i));
            from = i + 1;
        }
    }
    if depth != 0 {
        return None;
    }
    result.push((from, t.len()));
    Some(result)
}
fn constraint(t: &[Token], i: usize, s: &str) -> bool {
    let pair = |a, b| t[i].is(s, a) && t.get(i + 1).is_some_and(|x| x.is(s, b));
    pair("NOT", "NULL")
        || pair("PRIMARY", "KEY")
        || [
            "CONSTRAINT",
            "UNIQUE",
            "CHECK",
            "REFERENCES",
            "COLLATE",
            "GENERATED",
        ]
        .iter()
        .any(|k| t[i].is(s, k))
}

fn column(t: &[Token], s: &str, gaps: &mut Vec<Value>) -> Option<Value> {
    if t.len() < 2 || !t[0].identifier(s) {
        return None;
    }
    let mut i = 1;
    let mut depth = 0;
    while i < t.len() {
        if depth == 0 && (constraint(t, i, s) || t[i].is(s, "NULL") || t[i].is(s, "DEFAULT")) {
            break;
        }
        if t[i].is(s, "(") {
            depth += 1;
        }
        if t[i].is(s, ")") {
            depth -= 1;
        }
        i += 1;
    }
    if i == 1 {
        return None;
    }
    let mut row = json!({"name":t[0].raw(s),"key":t[0].key(s),"type":&s[t[1].a..t[i-1].b],"typeSource":range(t[1].a,t[i-1].b),"source":range(t[0].a,t.last().unwrap().b),"declaredNullability":"UNSPECIFIED","defaultExpression":null,"defaultSource":null,"primaryKeyDeclared":false});
    while i < t.len() {
        if t[i].is(s, "NOT") && t.get(i + 1).is_some_and(|x| x.is(s, "NULL")) {
            row["declaredNullability"] = json!("NOT_NULL");
            i += 2;
        } else if t[i].is(s, "NULL") {
            row["declaredNullability"] = json!("NULL");
            i += 1;
        } else if t[i].is(s, "PRIMARY") && t.get(i + 1).is_some_and(|x| x.is(s, "KEY")) {
            row["primaryKeyDeclared"] = json!(true);
            i += 2;
        } else if t[i].is(s, "CONSTRAINT") && t.get(i + 1).is_some_and(|x| x.identifier(s)) {
            i += 2;
            if i == t.len() {
                gap(gaps, "EMPTY_NAMED_CONSTRAINT", t[i - 2].a, t[i - 1].b);
            }
        } else if t[i].is(s, "DEFAULT") {
            let from = i + 1;
            i = from;
            let (mut depth, mut cases) = (0usize, 0usize);
            while i < t.len() {
                if i > from
                    && depth == 0
                    && cases == 0
                    && (constraint(t, i, s) || t[i].is(s, "NULL") || t[i].is(s, "DEFAULT"))
                    && !t[i - 1].is(s, "IS")
                    && !(i >= 2 && t[i - 1].is(s, "NOT") && t[i - 2].is(s, "IS"))
                {
                    break;
                }
                if t[i].is(s, "CASE") {
                    cases += 1;
                }
                if t[i].is(s, "END") {
                    cases = cases.saturating_sub(1);
                }
                if t[i].is(s, "(") || t[i].is(s, "[") {
                    depth += 1;
                }
                if t[i].is(s, ")") || t[i].is(s, "]") {
                    depth = depth.saturating_sub(1);
                }
                i += 1;
            }
            if from == i {
                gap(
                    gaps,
                    "EMPTY_DEFAULT_EXPRESSION",
                    t[from - 1].a,
                    t.last().unwrap().b,
                );
            } else {
                if !row["defaultExpression"].is_null() {
                    gap(
                        gaps,
                        "REPEATED_DEFAULT_DECLARATION",
                        t[from - 1].a,
                        t[i - 1].b,
                    );
                }
                row["defaultExpression"] = json!(&s[t[from].a..t[i - 1].b]);
                row["defaultSource"] = range(t[from].a, t[i - 1].b);
            }
        } else {
            gap(
                gaps,
                "COLUMN_CONSTRAINT_NOT_MODELED",
                t[i].a,
                t.last().unwrap().b,
            );
            break;
        }
    }
    Some(row)
}

fn table(
    t: &[Token],
    s: &str,
    database: &Option<String>,
    search_path: &Value,
    gaps: &mut Vec<Value>,
) -> Option<Value> {
    let mut at = 1;
    if t.get(at)
        .is_some_and(|x| x.is(s, "TEMP") || x.is(s, "TEMPORARY") || x.is(s, "UNLOGGED"))
    {
        at += 1;
    }
    if !t.get(at).is_some_and(|x| x.is(s, "TABLE")) {
        return None;
    }
    at += 1;
    if t.get(at).is_some_and(|x| x.is(s, "IF")) {
        if t.get(at + 1).is_none_or(|x| !x.is(s, "NOT"))
            || t.get(at + 2).is_none_or(|x| !x.is(s, "EXISTS"))
        {
            return None;
        }
        at += 3;
    }
    let first = at;
    if t.get(at).is_none_or(|x| !x.identifier(s)) {
        return None;
    }
    at += 1;
    while t.get(at).is_some_and(|x| x.is(s, ".")) {
        if t.get(at + 1).is_none_or(|x| !x.identifier(s)) {
            return None;
        }
        at += 2;
    }
    let name = &s[t[first].a..t[at - 1].b];
    if t.get(at).is_none_or(|x| !x.is(s, "(")) {
        return None;
    }
    let open = at;
    let mut depth = 1usize;
    at += 1;
    while at < t.len() && depth > 0 {
        if t[at].is(s, "(") {
            depth += 1;
        }
        if t[at].is(s, ")") {
            depth -= 1;
        }
        if depth > 32 {
            return None;
        }
        at += 1;
    }
    if depth > 0 {
        return None;
    }
    let close = at - 1;
    if at < t.len() {
        gap(
            gaps,
            "TABLE_SUFFIX_NOT_MODELED",
            t[at].a,
            t.last().unwrap().b,
        );
    }
    let mut columns = Vec::new();
    let mut keys: Vec<Vec<String>> = Vec::new();
    let mut declarations = Vec::new();
    for (a, b) in parts(&t[open + 1..close], s)? {
        let p = &t[open + 1 + a..open + 1 + b];
        if p.is_empty() {
            gap(gaps, "EMPTY_TABLE_ELEMENT", t[open].a, t[close].b);
            continue;
        }
        let mut start = 0;
        if p[0].is(s, "CONSTRAINT") && p.get(1).is_some_and(|x| x.identifier(s)) {
            start = 2;
        }
        if p.get(start).is_some_and(|x| x.is(s, "PRIMARY"))
            && p.get(start + 1).is_some_and(|x| x.is(s, "KEY"))
        {
            let k = &p[start + 2..];
            if k.first().is_some_and(|x| x.is(s, "(")) && k.last().is_some_and(|x| x.is(s, ")")) {
                let mut names = Vec::new();
                let mut valid = true;
                for (a, b) in parts(&k[1..k.len() - 1], s)? {
                    let ids = &k[1 + a..1 + b];
                    if ids.len() != 1 || !ids[0].identifier(s) {
                        valid = false;
                        break;
                    }
                    names.push(ids[0].key(s));
                }
                if valid && !names.is_empty() {
                    keys.push(names);
                    declarations.push(range(p[0].a, p.last().unwrap().b));
                    continue;
                }
            }
            gap(
                gaps,
                "PRIMARY_KEY_CLAUSE_NOT_MODELED",
                p[0].a,
                p.last().unwrap().b,
            );
        } else if start > 0
            || ["CHECK", "FOREIGN", "UNIQUE", "LIKE", "EXCLUDE"]
                .iter()
                .any(|k| p[0].is(s, k))
        {
            gap(
                gaps,
                "TABLE_CONSTRAINT_NOT_MODELED",
                p[0].a,
                p.last().unwrap().b,
            );
        } else if let Some(c) = column(p, s, gaps) {
            if columns.len() >= 256 {
                gap(gaps, "COLUMN_BOUND", p[0].a, t[close].a);
                break;
            }
            if c["primaryKeyDeclared"] == true {
                keys.push(vec![c["key"].as_str().unwrap().into()]);
                declarations.push(c["source"].clone());
            }
            columns.push(c);
        } else {
            gap(
                gaps,
                "TABLE_ELEMENT_NOT_MODELED",
                p[0].a,
                p.last().unwrap().b,
            );
        }
    }
    let names = columns
        .iter()
        .filter_map(|c| c["key"].as_str())
        .collect::<Vec<_>>();
    let valid = keys.len() == 1
        && keys[0]
            .iter()
            .all(|k| names.iter().filter(|n| **n == k).count() == 1)
        && {
            let mut unique = std::collections::BTreeSet::new();
            keys[0].iter().all(|k| unique.insert(k))
        };
    if !keys.is_empty() && !valid {
        gap(
            gaps,
            "PRIMARY_KEY_BINDING_NOT_ESTABLISHED",
            t[0].a,
            t.last().unwrap().b,
        );
    }
    for c in &mut columns {
        let pk = valid && keys[0].iter().any(|k| c["key"] == k.as_str());
        c["primaryKey"] = json!(pk);
        c["nullabilityBasis"] = json!(if c["declaredNullability"] == "NOT_NULL" {
            "DECLARED_NOT_NULL"
        } else if pk {
            "PRIMARY_KEY_RULE"
        } else if c["declaredNullability"] == "NULL" {
            "DECLARED_NULL"
        } else {
            "NOT_EXPLICITLY_DECLARED"
        });
        c["acceptsNull"] = if pk || c["declaredNullability"] == "NOT_NULL" {
            json!(false)
        } else if c["declaredNullability"] == "NULL" {
            json!(true)
        } else {
            Value::Null
        };
    }
    Some(
        json!({"name":name,"database":database,"searchPath":search_path,"source":range(t[0].a,t.last().unwrap().b),"columns":columns,"primaryKeyDeclarations":keys,"primaryKeySources":declarations,"primaryKeyBindingEstablished":valid}),
    )
}

#[derive(Clone)]
struct Section {
    database: Option<String>,
    start: usize,
    end: usize,
}
pub(super) fn project(s: &str) -> Value {
    project_with_sections(s).0
}
fn project_with_sections(s: &str) -> (Value, Vec<Section>) {
    let (t, mut frontier) = lex(s);
    let mut tables = Vec::new();
    let mut i = 0;
    let mut database = None;
    let mut sections = Vec::new();
    let mut section_start = 0;
    let mut search_path = Value::Null;
    while i < t.len() {
        if tables.len() >= 64 {
            gap(&mut frontier, "TABLE_BOUND", t[i].a, s.len());
            break;
        }
        if t[i].is(s, "\\") {
            sections.push(Section {
                database: database.clone(),
                start: section_start,
                end: t[i].a,
            });
            section_start = t[i].a;
            let end = s[t[i].a..].find('\n').map_or(s.len(), |a| t[i].a + a);
            let mut z = i;
            while z < t.len() && t[z].a < end {
                z += 1;
            }
            let p = &t[i..z];
            if p.len() == 3 && (p[1].is(s, "c") || p[1].is(s, "connect")) && p[2].identifier(s) {
                database = Some(p[2].key(s));
                search_path = Value::Null;
            } else {
                database = None;
                search_path = Value::Null;
                gap(
                    &mut frontier,
                    "PSQL_COMMAND_CONTEXT_NOT_MODELED",
                    t[i].a,
                    end,
                );
            }
            i = z;
            continue;
        }
        let from = i;
        let mut depth = 0usize;
        while i < t.len() {
            if t[i].is(s, ";") && depth == 0 {
                break;
            }
            if t[i].is(s, "(") {
                depth += 1;
            }
            if t[i].is(s, ")") {
                depth = depth.saturating_sub(1);
            }
            i += 1;
        }
        let p = &t[from..i];
        i += usize::from(i < t.len());
        if p.is_empty() {
            continue;
        }
        if p[0].is(s, "CREATE") {
            if p.get(1).is_some_and(|x| x.is(s, "DATABASE")) && p.len() == 3 && p[2].identifier(s) {
                continue;
            }
            if let Some(value) = table(p, s, &database, &search_path, &mut frontier) {
                tables.push(value);
                continue;
            }
        }
        if p.len() >= 4
            && p[0].is(s, "SET")
            && p[1].is(s, "search_path")
            && (p[2].is(s, "TO") || p[2].is(s, "="))
        {
            let mut names = Vec::new();
            let mut valid = true;
            for (a, b) in parts(&p[3..], s).unwrap_or_default() {
                let v = &p[3 + a..3 + b];
                if v.len() != 1 || !v[0].identifier(s) {
                    valid = false;
                    break;
                }
                names.push(v[0].key(s));
            }
            if valid && !names.is_empty() {
                search_path = json!(names);
                continue;
            }
            search_path = Value::Null;
        }
        gap(
            &mut frontier,
            "STATEMENT_NOT_MODELED",
            p[0].a,
            p.last().unwrap().b,
        );
    }
    sections.push(Section {
        database,
        start: section_start,
        end: s.len(),
    });
    (
        json!({"schema":"codeclew-postgres-create-table/1.0","authority":"DECLARED_DDL_NOT_DATABASE_STATE_OR_EXECUTION","tables":tables,"frontier":frontier,"contextMeaning":"Declared script database/search_path context, not observed connection state","supportedGrammar":"CREATE [TEMP|TEMPORARY|UNLOGGED] TABLE [IF NOT EXISTS] qualified_identifier (columns, [CONSTRAINT name] PRIMARY KEY(columns)); exact types/default expressions; NULL/NOT NULL; psql connect and SET search_path. Other statements/constraints remain explicit frontiers."}),
        sections,
    )
}

/// Select complete declared database sections in the original coordinate space.
/// No source slicing, parser replacement or ownership/dependency inference.
pub(super) fn project_scoped(
    s: &str,
    scope: &super::SqlScope,
) -> Result<Value, crate::error::ClewError> {
    use super::super::invalid;
    let (mut projected, sections) = project_with_sections(s);
    // Unknown constructs can hide cross-section dependencies or boundaries.
    // Refuse narrowing; the unchanged full projection remains available.
    if !projected["frontier"].as_array().unwrap().is_empty() || sections.len() > 64 {
        return Err(invalid(
            "SQL scope cannot exclude unresolved declarations, dependencies or boundaries; use the full unscoped projection and its frontier",
        ));
    }
    let expected = sections
        .iter()
        .filter(|section| section.database.as_deref() == Some(scope.database.as_str()))
        .map(|section| super::SqlRange {
            coordinate_domain: "SOURCE_RECORD_UTF8_BYTES".into(),
            byte_start: section.start,
            byte_end: section.end,
        })
        .collect::<Vec<_>>();
    if expected.is_empty()
        || scope.ranges != expected
        || scope.ranges.iter().any(|r| {
            r.byte_start >= r.byte_end
                || !s.is_char_boundary(r.byte_start)
                || !s.is_char_boundary(r.byte_end)
        })
    {
        return Err(invalid(
            "SQL scope must match every complete original section of the selected database in source order",
        ));
    }
    let tables = projected["tables"].as_array().unwrap();
    let mut included_sections = Vec::new();
    let mut excluded_sections = Vec::new();
    let mut included = Vec::new();
    let mut accounted = 0;
    for section in &sections {
        let rows = tables
            .iter()
            .filter(|table| {
                let start = table["source"]["byteStart"].as_u64().unwrap() as usize;
                let end = table["source"]["byteEnd"].as_u64().unwrap() as usize;
                start >= section.start && end <= section.end
            })
            .cloned()
            .collect::<Vec<_>>();
        accounted += rows.len();
        if !rows.is_empty() && section.database.is_none() {
            return Err(invalid(
                "SQL scope cannot classify tables with unknown database context as excluded",
            ));
        }
        let selected = section.database.as_deref() == Some(scope.database.as_str());
        let record = json!({"database":section.database,"source":range(section.start,section.end),"tables":rows,
            "classification":if selected {"INCLUDED_EXPLICIT_DATABASE_SECTION"} else if section.database.is_some() {"EXCLUDED_DISTINCT_EXPLICIT_DATABASE_SECTION"} else {"RETAINED_PREAMBLE_WITHOUT_TABLES"}});
        if selected {
            included.extend(rows);
            included_sections.push(record);
        } else {
            excluded_sections.push(record);
        }
    }
    if accounted != tables.len() || included.is_empty() {
        return Err(invalid(
            "SQL scope has unclassified or no selected table declarations",
        ));
    }
    let total = tables.len();
    projected["tables"] = json!(included);
    projected["scope"] = json!({"schema":"codeclew-declared-database-scope/1.0","selection":scope,
        "includedSections":included_sections,"excludedSections":excluded_sections,"originalTableCount":total,
        "unresolvedFrontier":projected["frontier"],"authority":"EXPLICIT_DECLARED_DATABASE_CONTEXTS_NOT_RUNTIME_OWNERSHIP_OR_UNUSED_DATA_PROOF",
        "basisAuthority":"Caller-selected rationale; evidence source identities authenticated separately; semantic relationship not inferred"});
    Ok(projected)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn database_scope(s: &str, database: &str) -> super::super::SqlScope {
        let (_, sections) = project_with_sections(s);
        serde_json::from_value(json!({"database":database,
            "ranges":sections.iter().filter(|v|v.database.as_deref()==Some(database)).map(|v|range(v.start,v.end)).collect::<Vec<_>>(),
            "reason":"Retained declared connection selects this database", "evidence":[]})).unwrap()
    }
    #[test]
    fn scope_preserves_original_offsets_and_same_named_tables_in_distinct_databases() {
        let s = "-- π prefix\nCREATE DATABASE a;\n\\connect a\nCREATE TABLE inbox (id uuid PRIMARY KEY, kind text);\n\\connect b\nCREATE TABLE inbox (id integer PRIMARY KEY);\n\\connect a\nCREATE TABLE outbox (id bigint PRIMARY KEY);";
        let scope = database_scope(s, "a");
        let full = project(s);
        let scoped = project_scoped(s, &scope).unwrap();
        assert_eq!(scoped["tables"].as_array().unwrap().len(), 2);
        assert_eq!(scoped["tables"][0], full["tables"][0]);
        assert_eq!(scoped["tables"][1], full["tables"][2]);
        assert_eq!(
            scoped["scope"]["excludedSections"][1]["tables"][0],
            full["tables"][1]
        );
        for table in scoped["tables"].as_array().unwrap() {
            let a = table["source"]["byteStart"].as_u64().unwrap() as usize;
            let b = table["source"]["byteEnd"].as_u64().unwrap() as usize;
            assert!(s[a..b].starts_with("CREATE TABLE"));
            assert_eq!(table["database"], "a");
        }
        assert_eq!(
            scoped["scope"]["includedSections"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(scoped["scope"]["originalTableCount"], 3);
        assert_eq!(scoped["scope"]["unresolvedFrontier"], json!([]));
        let mut bad = scope.clone();
        bad.ranges.pop();
        assert!(project_scoped(s, &bad).is_err());
        let mut bad = scope.clone();
        bad.database = "b".into();
        assert!(project_scoped(s, &bad).is_err());
        for start in [
            scope.ranges[0].byte_start + 1,
            full["tables"][0]["source"]["byteStart"].as_u64().unwrap() as usize,
            usize::MAX,
        ] {
            let mut bad = scope.clone();
            bad.ranges[0].byte_start = start;
            assert!(project_scoped(s, &bad).is_err());
        }
        let mut bad = scope;
        bad.ranges[0].coordinate_domain = "FILE_LINES".into();
        assert!(project_scoped(s, &bad).is_err());
    }
    #[test]
    fn scope_refuses_unknown_context_dependencies_and_uncertain_boundaries() {
        let variants = [
            "CREATE TABLE orphan (id int);\n\\c a\nCREATE TABLE inbox (id int);\n\\c b\nCREATE TABLE inbox (id int);",
            "\\c a\nCREATE TABLE inbox (id int REFERENCES external(id));\n\\c b\nCREATE TABLE inbox (id int);",
            "\\c a\nCREATE TABLE inbox (id int);\n\\c b\nALTER TABLE inbox ADD CHECK(id>0);",
            "\\c a\nCREATE TABLE inbox (id int);\n\\include other.sql\n",
            "\\c a\nCREATE TABLE inbox (id int);\n\\c b\nCREATE TABLE bad (x text DEFAULT 'unfinished",
        ];
        for s in variants {
            let scope = database_scope(s, "a");
            assert!(project_scoped(s, &scope).is_err(), "{s}");
            // A refused selection never consumes or prunes the full frontier/source.
            let full = project(s);
            assert!(!full["tables"].as_array().unwrap().is_empty());
        }
    }
    #[test]
    fn exact_type_default_ranges_and_trailing_null_clause_are_retained() {
        let s = "CREATE TABLE sample (a numeric(4, 2) DEFAULT NULL NULL, b boolean DEFAULT 'x' IS NOT NULL NOT NULL);";
        let p = project(s);
        assert_eq!(p["frontier"], json!([]));
        let rows = p["tables"][0]["columns"].as_array().unwrap();
        assert_eq!(rows[0]["defaultExpression"], "NULL");
        assert_eq!(rows[0]["declaredNullability"], "NULL");
        assert_eq!(rows[1]["defaultExpression"], "'x' IS NOT NULL");
        assert_eq!(rows[1]["declaredNullability"], "NOT_NULL");
        for row in rows {
            for (field, span) in [
                ("type", "typeSource"),
                ("defaultExpression", "defaultSource"),
            ] {
                let a = row[span]["byteStart"].as_u64().unwrap() as usize;
                let b = row[span]["byteEnd"].as_u64().unwrap() as usize;
                assert_eq!(&s[a..b], row[field].as_str().unwrap());
            }
        }
    }
    #[test]
    fn keys_types_defaults_quotes_and_context_are_declarations() {
        let s = "-- CREATE TABLE fake(x int);\n\\connect alpha\nSET search_path TO \"custom schema\"; CREATE TABLE \"Odd.Table\" (\"Key,A\" uuid PRIMARY KEY, amount numeric(10, 2) NOT NULL DEFAULT (round(1.25, 1)), note text NULL DEFAULT 'a,b);--x', stamp timestamp with time zone DEFAULT now());\n\\c beta\nCREATE TABLE public.other (a int, b bigint, CONSTRAINT key_pair PRIMARY KEY(a,b));";
        let p = project(s);
        assert_eq!(p["tables"].as_array().unwrap().len(), 2);
        assert_eq!(p["frontier"], json!([]));
        assert_eq!(p["tables"][0]["database"], "alpha");
        assert_eq!(p["tables"][1]["database"], "beta");
        let c = &p["tables"][0]["columns"];
        assert_eq!(c[0]["nullabilityBasis"], "PRIMARY_KEY_RULE");
        assert_eq!(c[1]["nullabilityBasis"], "DECLARED_NOT_NULL");
        assert_eq!(c[1]["type"], "numeric(10, 2)");
        assert_eq!(c[2]["defaultExpression"], "'a,b);--x'");
        assert_eq!(c[3]["type"], "timestamp with time zone");
        assert_eq!(p["tables"][1]["columns"][1]["primaryKey"], true);
    }
    #[test]
    fn unsupported_constraints_statements_and_lexical_gaps_remain_visible() {
        let s = "CREATE TABLE a (id int, value text CHECK (value <> 'PRIMARY KEY'), FOREIGN KEY(id) REFERENCES b(id)); ALTER TABLE a ADD PRIMARY KEY(id); CREATE TABLE b (id int DEFAULT $$CREATE TABLE not_real(x int);$$); /* never closed";
        let p = project(s);
        assert_eq!(p["tables"].as_array().unwrap().len(), 2);
        assert!(p["frontier"].as_array().unwrap().len() >= 4);
        assert_eq!(p["tables"][0]["columns"][0]["primaryKey"], false);
        assert_eq!(p["tables"][0]["columns"][0]["acceptsNull"], Value::Null);
        assert_eq!(
            p["tables"][1]["columns"][0]["defaultExpression"],
            "$$CREATE TABLE not_real(x int);$$"
        );
    }
    #[test]
    fn nested_defaults_bom_comments_and_same_named_tables_stay_distinct() {
        let s = "\u{feff}/* outer /* inner */ end */\n\\c first\nCREATE TABLE schema_x.item (id int, a int[] DEFAULT ARRAY[1,2], b text DEFAULT E'it\\'s,ok', c int DEFAULT CASE WHEN true THEN 1 ELSE NULL END, PRIMARY KEY(id));\n\\connect second\nCREATE TABLE schema_x.item (id int NULL PRIMARY KEY);";
        let p = project(s);
        assert_eq!(p["frontier"], json!([]));
        let tables = p["tables"].as_array().unwrap();
        assert_eq!(tables.len(), 2);
        assert_ne!(tables[0]["database"], tables[1]["database"]);
        let c = &tables[0]["columns"];
        assert_eq!(c[1]["defaultExpression"], "ARRAY[1,2]");
        assert_eq!(c[2]["defaultExpression"], "E'it\\'s,ok'");
        assert_eq!(
            c[3]["defaultExpression"],
            "CASE WHEN true THEN 1 ELSE NULL END"
        );
        assert_eq!(tables[1]["columns"][0]["declaredNullability"], "NULL");
        assert_eq!(
            tables[1]["columns"][0]["nullabilityBasis"],
            "PRIMARY_KEY_RULE"
        );
        for table in tables {
            for col in table["columns"].as_array().unwrap() {
                let a = col["source"]["byteStart"].as_u64().unwrap() as usize;
                let z = col["source"]["byteEnd"].as_u64().unwrap() as usize;
                assert!(s.get(a..z).is_some());
            }
        }
    }
    #[test]
    fn unresolved_key_and_bounds_do_not_assert_key_nullability() {
        let p = project("CREATE TABLE neutral (x int, PRIMARY KEY(missing));");
        assert_eq!(p["tables"][0]["columns"][0]["primaryKey"], false);
        assert_eq!(p["tables"][0]["columns"][0]["acceptsNull"], Value::Null);
        assert!(!p["frontier"].as_array().unwrap().is_empty());
        let many = format!(
            "CREATE TABLE bounded ({});",
            (0..260)
                .map(|i| format!("c{i} int"))
                .collect::<Vec<_>>()
                .join(",")
        );
        let p = project(&many);
        assert_eq!(p["tables"][0]["columns"].as_array().unwrap().len(), 256);
        assert!(
            p["frontier"]
                .as_array()
                .unwrap()
                .iter()
                .any(|g| g["code"] == "COLUMN_BOUND")
        );
    }
}
