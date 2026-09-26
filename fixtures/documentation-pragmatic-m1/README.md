# Pragmatic M1 documentation corpus

This frozen, source-available fixture is a separate copy of the Java 17 orders
shape used by `fixtures/durable-docs/orders`. Its selected journey is
`CheckoutController.checkout` through a helper guard, a repository interface
call, and `InventoryClient.reserve`. It adds two nearby negative controls: a
save callback that is created but never run, and a Spring `RestClient` request
spec that is constructed without an execution call. The sibling `GET /ready`
operation and `notes/checkout.md` are preservation controls for later update
work.

The architecture files follow the public documentation bundle layout used in
`fixtures/durable-docs/architecture`; the source project is under `orders/`.
The author brief contains questions only. Expected answers and exact evaluator
references live in `evaluation/reader-obligations.json` and must not be passed
as author input.

`evaluation/corpus-lock.json` pins the fixture files to base source revision
`0c413af7e8cd947cdd823491aed1afce63d5c689` and records per-file SHA-256 hashes.
Its aggregate digest hashes the UTF-8 lines `<sha256>  <relative-path>\n` in
lexical path order, excluding the lock file itself.

## Checks and limits

The fixture targets Java 17 and Spring Web 6.1.13. Run its syntax and dependency
API check from the repository root with:

```sh
mvn -o -f fixtures/documentation-pragmatic-m1/orders/pom.xml -DskipTests compile
```

This compile check exercises the actual Spring 6.1.13 `RestClient` API available
to Maven. It does not run Spring, instantiate the controller, execute either
HTTP client, or make a network call. `ReservationRepository` has no
implementation, so its `save` call does not establish a persistent write or
commit. The source identifies the RestTemplate method, path suffix, and
configuration key, but cannot establish a concrete configured host, delivery,
remote handling, or remote completion. A request spec without `retrieve` and
an execution method remains construction only. Static source evidence also
does not prove that any request value reaches the happy path at runtime.
