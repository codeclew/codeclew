//! Equal semantic context is insufficient to continue across saved source snapshots.
#![cfg(unix)]
#[path = "support/documentation.rs"]
mod support;
use clew::documentation::store::Repository;
use support::Fixture;

#[test]
fn context_cursor_binds_resolved_snapshot_with_same_semantic_digest() {
    let f = Fixture::new();
    f.service("orders");
    let repo = Repository::open(&f.docs).unwrap();
    let mut check = f.checked();
    let a = check.save_snapshot(&repo).unwrap();
    let first = f.ok(&["docs", "context", "--service", "orders", "--limit", "1"]);
    let cursor = first["nextCursor"]
        .as_str()
        .expect("multiple source context rows");
    for source in check
        .services
        .get_mut("orders")
        .unwrap()
        .sources
        .values_mut()
    {
        source.start_line += 10;
        source.end_line += 10;
    }
    let b = check.save_snapshot(&repo).unwrap();
    assert_ne!(a, b);
    assert_ne!(
        f.run(&[
            "docs",
            "context",
            "--service",
            "orders",
            "--limit",
            "1",
            "--cursor",
            cursor
        ])
        .0,
        0
    );
    assert_ne!(
        f.run(&[
            "docs",
            "context",
            "--service",
            "orders",
            "--snapshot",
            &b,
            "--limit",
            "1",
            "--cursor",
            cursor
        ])
        .0,
        0
    );
    let resumed = f.ok(&[
        "docs",
        "context",
        "--service",
        "orders",
        "--snapshot",
        &a,
        "--limit",
        "1",
        "--cursor",
        cursor,
    ]);
    assert_eq!(resumed["snapshot"], a);
    assert_eq!(resumed["contextDigest"], first["contextDigest"]);
}

#[test]
fn changes_cursor_cannot_cross_snapshots_with_equal_context_digest() {
    let f = Fixture::new();
    let source = f.service("orders");
    let repo = Repository::open(&f.docs).unwrap();
    let mut check = f.checked();
    let narrative = f.author("orders", &check);
    f.ok(&["docs", "render", "--input", narrative.to_str().unwrap()]);
    let path = source.join("Orders.java");
    std::fs::write(
        &path,
        std::fs::read_to_string(&path)
            .unwrap()
            .replace("return quantity;", "return quantity + 1;"),
    )
    .unwrap();
    support::commit(&source);
    check = f.checked();
    let a = check.save_snapshot(&repo).unwrap();
    let first = f.ok(&["docs", "changes", "--limit", "1"]);
    let cursor = first["nextCursor"]
        .as_str()
        .expect("multiple change dossier rows");
    for source in check
        .services
        .get_mut("orders")
        .unwrap()
        .sources
        .values_mut()
    {
        source.start_line += 20;
        source.end_line += 20;
    }
    let b = check.save_snapshot(&repo).unwrap();
    assert_ne!(a, b);
    assert_ne!(
        f.run(&["docs", "changes", "--limit", "1", "--cursor", cursor])
            .0,
        0
    );
    assert_ne!(
        f.run(&[
            "docs",
            "changes",
            "--snapshot",
            &b,
            "--limit",
            "1",
            "--cursor",
            cursor
        ])
        .0,
        0
    );
    let resumed = f.ok(&[
        "docs",
        "changes",
        "--snapshot",
        &a,
        "--limit",
        "1",
        "--cursor",
        cursor,
    ]);
    assert_eq!(resumed["snapshot"], a);
    assert_eq!(resumed["contextDigest"], first["contextDigest"]);
}
