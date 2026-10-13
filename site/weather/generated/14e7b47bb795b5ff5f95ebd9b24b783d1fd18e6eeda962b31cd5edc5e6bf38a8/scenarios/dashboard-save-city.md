# Dashboard save-city thread

Static source interpretation. Declared interactions do not establish runtime routing.

Source freshness: UNVERIFIED. Meaning review: UNASSESSED.

Content revisions: {"dashboard":"e635069293d58cb0d4f28c8211e748b8d58e6b37","forecasts":"c289b28be4819eddf29d59a7c12f178559ec5229","places":"c289b28be4819eddf29d59a7c12f178559ec5229"}

Target revisions: {"dashboard":"e635069293d58cb0d4f28c8211e748b8d58e6b37","forecasts":"c289b28be4819eddf29d59a7c12f178559ec5229","places":"c289b28be4819eddf29d59a7c12f178559ec5229"}

## Save a city through Dashboard and record the outbox boundary

A caller submits a cityId to Dashboard.savePlace. The handler serializes the body and sends POST /places with a 10-second abort signal; the declared OpenAPI request is an object containing only cityId, with oslo, helsinki, and lisbon as allowed values. The saved Dashboard-to-Places mapping is agent-proposed, so runtime target binding is not established. Places returns 400 UNSUPPORTED_CITY when its city mapping fails; otherwise it awaits repository.Save and returns 200. Save inserts the place in a database transaction, inserts a place.requested version 1 outbox record only when the place insert returns a new id, and commits. A conflict skips the outbox insert but still commits. Dashboard forwards a parsed upstream status/body, while a fetch or JSON parse exception returns 503 PLACES_UNAVAILABLE; the wrapper has no retry and that response does not say whether the Places transaction committed. The no-outbox path is tied to the place INSERT returning null on conflict; that can reflect an existing city or another request, so it is not a retry guarantee for the timed-out caller. The saved process links publisher and consumer views separately; an outbox row is not proof of Kafka publication or consumption.

A caller enters savePlace with a JSON body. The declared Places request body requires only cityId, a string in the oslo, helsinki, or lisbon enum; SavePlace exposes that value as CityId.

Dashboard POSTs JSON.stringify(request.body) to /places with content-type application/json and AbortSignal.timeout(10000). The saved interaction is an agent-proposed target mapping, not proof of runtime routing.

Dashboard fetch and response.json complete

PlacesRepository.City returns null for CityId

Places returns HTTP 400 with error UNSUPPORTED_CITY.

The source describes an alternative branch.

For a supported city, repository.Save opens a connection, begins a transaction, and inserts the place with ON CONFLICT DO NOTHING RETURNING id.

The place INSERT returns a new id

Within the same transaction, InsertRequest creates a UUID and inserts a JSONB outbox payload with type place.requested, version 1, and the place.

The source describes an alternative branch.

If the INSERT returns null after a conflict, InsertRequest is skipped, so this call adds no outbox row.

repository.Save commits the transaction after the optional outbox insert; the new place and its outbox row share that transaction.

After repository.Save returns, the controller returns HTTP 200 with the mapped city.

Dashboard forwards the upstream status and parsed JSON body. The shown Places branches return 400 for an unsupported city or 200 after a supported save.

The source describes an alternative branch.

A fetch or response.json exception, including an abort from the 10-second signal, is caught and mapped to HTTP 503 PLACES_UNAVAILABLE.

The exception may occur before or after Places commits, so this 503 is not a database outcome. The handler has no retry branch. A later call can follow the no-outbox conflict path whenever the place INSERT returns null, including for a preexisting city or another request; the source does not identify which request caused that state.

The saved process links the Places publisher, Forecasts consumer/publisher, and Dashboard consumer views as separate subviews. This HTTP narrative does not establish their activation, routing, retry behavior, or message delivery, and the outbox insert is not a synchronous broker acknowledgement.

### Declared Places POST /places contract

Source-derived http interface description.

| Element | Value / behavior |
|---|---|
| Request | The declared required JSON body is an object with no additional properties and one required cityId string, restricted to oslo, helsinki, or lisbon. |
| Declared responses | The OpenAPI operation declares a 200 Place JSON object and a 400 JSON error object, plus 415 and 500 status codes. The captured controller explicitly returns 400 for an unsupported mapping and 200 after repository.Save; the declaration does not prove runtime enforcement or exception-to-status mapping. |
| Routing boundary | The saved process records Dashboard.savePlace to Places.Save as an agent-proposed POST /places interaction. Runtime receiver binding and routing remain unverified. |

<details>
<summary>Implementation details</summary>

```mermaid
sequenceDiagram
    autonumber
    %% Agent-interpreted static source; declared edges are not compiler calls.
    participant caller as Caller
    participant service-dashboard as dashboard
    participant places as Places
    participant places-db as Places PostgreSQL
    %% claim-867c3a6ea923dd9ac3b0: dependencies dashboard:symbol:7970ac3e2a8cf3f45b4a6217, places:contract-operation:08d1b9df6781b2953a038bc2, places:contract-operation:0f29e92a853c76a6ea564769, places:contract-operation:6b165729e2d309a9b2687873, places:contract-operation:6e713b9442ae5dfc4b99b133, places:contract-operation:fa5644a96e01681d7c2b0445, places:contract:91b280ed0ed0eda3e8e3231d, places:symbol:6c7321d96988c6277e32e39e
    caller->>service-dashboard: A caller enters savePlace with a JSON body. The declared Places request body requires only cityId, a string in the oslo, helsinki, or lisbon enum&#59; SavePlace exposes that value as CityId.
    %% claim-ea66fc67117c852a5204: dependencies dashboard:symbol:7970ac3e2a8cf3f45b4a6217, interaction:dashboard-save-place-to-places, scenario:dashboard-save-city
    service-dashboard->>places: Dashboard POSTs JSON.stringify(request.body) to /places with content-type application/json and AbortSignal.timeout(10000). The saved interaction is an agent-proposed target mapping, not proof of runtime routing.
    %% claim-4fb435287aa6213d6050: dependencies dashboard:flow:8fbb54c4bae55833802a2fce, dashboard:symbol:7970ac3e2a8cf3f45b4a6217, scenario:dashboard-save-city
    alt Dashboard fetch and response.json complete
    %% claim-2ffb3392163a7f4d7efe: dependencies places:entrypoint:17bfec4e1ad29c7ec0342f46, places:symbol:309961f88b197290c1b04e50, scenario:dashboard-save-city
    alt PlacesRepository.City returns null for CityId
    %% claim-9598ebe173333356a7d5: dependencies places:contract-operation:fa5644a96e01681d7c2b0445, places:entrypoint:17bfec4e1ad29c7ec0342f46, places:symbol:309961f88b197290c1b04e50
    places-->>service-dashboard: Places returns HTTP 400 with error UNSUPPORTED_CITY.
    %% claim-fda7a7fc7f2484b9f443: dependencies places:entrypoint:17bfec4e1ad29c7ec0342f46, places:symbol:309961f88b197290c1b04e50, scenario:dashboard-save-city
    else Otherwise
    %% claim-206662371b21832b8908: dependencies places:entrypoint:17bfec4e1ad29c7ec0342f46, places:symbol:309961f88b197290c1b04e50, places:symbol:d3b23b700626394c74b7ef9e, scenario:dashboard-save-city
    places->>places-db: For a supported city, repository.Save opens a connection, begins a transaction, and inserts the place with ON CONFLICT DO NOTHING RETURNING id.
    %% claim-b80eef221dfda271fc4d: dependencies places:symbol:d3b23b700626394c74b7ef9e, scenario:dashboard-save-city
    alt The place INSERT returns a new id
    %% claim-c3f082c7c33e8cbbb7cd: dependencies places:symbol:51d94b76d870001475fd2c51, places:symbol:d3b23b700626394c74b7ef9e, scenario:dashboard-save-city
    places->>places-db: Within the same transaction, InsertRequest creates a UUID and inserts a JSONB outbox payload with type place.requested, version 1, and the place.
    %% claim-e52f5ff3c423edff90bc: dependencies places:symbol:d3b23b700626394c74b7ef9e, scenario:dashboard-save-city
    else Otherwise
    %% claim-7dc7e55eff21ec3ff190: dependencies places:symbol:d3b23b700626394c74b7ef9e, scenario:dashboard-save-city
    Note over caller: If the INSERT returns null after a conflict, InsertRequest is skipped, so this call adds no outbox row.
    %% claim-d573e35cc6c11b6abc5a: dependencies places:symbol:d3b23b700626394c74b7ef9e, scenario:dashboard-save-city
    end
    %% claim-7ec326c9475caf47aec9: dependencies places:symbol:51d94b76d870001475fd2c51, places:symbol:d3b23b700626394c74b7ef9e, scenario:dashboard-save-city
    places->>places-db: repository.Save commits the transaction after the optional outbox insert&#59; the new place and its outbox row share that transaction.
    %% claim-b2c2a256aeb68b422a06: dependencies places:contract-operation:fa5644a96e01681d7c2b0445, places:entrypoint:17bfec4e1ad29c7ec0342f46, places:symbol:309961f88b197290c1b04e50, scenario:dashboard-save-city
    places-->>service-dashboard: After repository.Save returns, the controller returns HTTP 200 with the mapped city.
    %% claim-1b063f26de1df2988c9f: dependencies places:entrypoint:17bfec4e1ad29c7ec0342f46, places:symbol:309961f88b197290c1b04e50, scenario:dashboard-save-city
    end
    %% claim-9b18a215fbe88c5c129a: dependencies dashboard:symbol:7970ac3e2a8cf3f45b4a6217, places:contract-operation:fa5644a96e01681d7c2b0445, places:entrypoint:17bfec4e1ad29c7ec0342f46, places:symbol:309961f88b197290c1b04e50, scenario:dashboard-save-city
    service-dashboard-->>caller: Dashboard forwards the upstream status and parsed JSON body. The shown Places branches return 400 for an unsupported city or 200 after a supported save.
    %% claim-fb0dd41468e0b1f39096: dependencies dashboard:flow:8fbb54c4bae55833802a2fce, dashboard:symbol:7970ac3e2a8cf3f45b4a6217, scenario:dashboard-save-city
    else Otherwise
    %% claim-62ecc1e2e5ebf8b783cf: dependencies dashboard:symbol:7970ac3e2a8cf3f45b4a6217
    service-dashboard-->>caller: A fetch or response.json exception, including an abort from the 10-second signal, is caught and mapped to HTTP 503 PLACES_UNAVAILABLE.
    %% claim-097b234b1c456213cf3d: dependencies dashboard:symbol:7970ac3e2a8cf3f45b4a6217, places:symbol:51d94b76d870001475fd2c51, places:symbol:d3b23b700626394c74b7ef9e, scenario:dashboard-save-city
    Note over caller: The exception may occur before or after Places commits, so this 503 is not a database outcome. The handler has no retry branch. A later call can follow the no-outbox conflict path whenever the place INSERT returns null, including for a preexisting city or another request&#59; the source does not identify which request caused that state.
    %% claim-08aed576a52a09ce13e2: dependencies dashboard:flow:8fbb54c4bae55833802a2fce, dashboard:symbol:7970ac3e2a8cf3f45b4a6217, scenario:dashboard-save-city
    end
    %% claim-264815f9722d1cf4918d: dependencies interaction:forecasts-forecast-updated-to-dashboard, interaction:places-place-requested-to-dashboard, interaction:places-place-requested-to-forecasts, places:symbol:51d94b76d870001475fd2c51, process:dashboard-save-city, scenario:dashboard-save-city
    Note over caller: The saved process links the Places publisher, Forecasts consumer/publisher, and Dashboard consumer views as separate subviews. This HTTP narrative does not establish their activation, routing, retry behavior, or message delivery, and the outbox insert is not a synchronous broker acknowledgement.
```

</details>

Source freshness: STALE. Meaning review: UNASSESSED.

Content revisions: {"dashboard":"e635069293d58cb0d4f28c8211e748b8d58e6b37","forecasts":"c289b28be4819eddf29d59a7c12f178559ec5229","places":"c289b28be4819eddf29d59a7c12f178559ec5229"}

Target revisions: {"dashboard":"e635069293d58cb0d4f28c8211e748b8d58e6b37","forecasts":"c289b28be4819eddf29d59a7c12f178559ec5229","places":"c289b28be4819eddf29d59a7c12f178559ec5229"}

## Dashboard save-city thread

savePlace posts the JSON request body to placesUrl/places with an application/json content type, waits up to ten seconds, and forwards the response status and JSON body. A fetch or JSON-decoding error returns 503 PLACES_UNAVAILABLE. The saved process links this request to the proposed Places.Save mapping and separate request-publisher and event-consumer views. The target service binding and network result are unverified; linked event views do not establish downstream processing, delivery or execution order.

<details>
<summary>Implementation details</summary>

```mermaid
sequenceDiagram
    autonumber
    %% Agent-interpreted static source; declared edges are not compiler calls.
```

</details>


## Saved process definition

Explicit requested scope and outcomes; source support is assessed separately.

Scope: Dashboard.savePlace and its proposed Places.Save mapping, linked to the place-requested publisher, Forecasts consumer/publisher, and Dashboard consumer views.

Trigger: A caller enters the source-declared Dashboard.savePlace handler.

participants:

- dashboard
- places
- forecasts

objects:


outcomes:

- The declared HTTP and Kafka relationships are retained as separate views; target resolution, network result and downstream delivery are not established here.

### Linked child views: captured versions

- [dashboard-forecast-updated-consumer](dashboard-forecast-updated-consumer.md): LINKED_PROCESS_STALE
- [dashboard-place-requested-consumer](dashboard-place-requested-consumer.md): LINKED_PROCESS_STALE
- [forecasts-place-requested-consumer](forecasts-place-requested-consumer.md): LINKED_PROCESS_STALE
- [forecasts-publish-pending](forecasts-publish-pending.md): LINKED_PROCESS_STALE
- [places-request-publisher](places-request-publisher.md): LINKED_PROCESS_STALE

## Process steps

Declared reading order does not prove execution order. Call targets and delivery are not established.

### Boundary contract: dashboard-save-http

Declared source interpretation; wire compatibility and runtime are unproven.

- operation: Dashboard sends POST /places to the declared Places.Save target.
  Evidence: dashboard / services/dashboard/src/main.ts @ e635069293d58cb0d4f28c8211e748b8d58e6b37 #L67-L74
  Evidence: places / services/places/PlacesController.cs @ c289b28be4819eddf29d59a7c12f178559ec5229 #L10-L17
- request: Dashboard serializes request.body as JSON and sets content-type to application/json. The body schema is not established by these two handler bodies.
  Evidence: dashboard / services/dashboard/src/main.ts @ e635069293d58cb0d4f28c8211e748b8d58e6b37 #L67-L74
- request: Places reads SavePlace.CityId; an unsupported mapped city returns 400 with UNSUPPORTED_CITY.
  Evidence: places / services/places/PlacesController.cs @ c289b28be4819eddf29d59a7c12f178559ec5229 #L10-L17
- response: For a supported mapped city, Places awaits repository.Save and then returns 200 with the city. The repository effect is not proven by the controller wrapper.
  Evidence: places / services/places/PlacesController.cs @ c289b28be4819eddf29d59a7c12f178559ec5229 #L10-L17
- response: Dashboard forwards the upstream status and JSON body after parsing the response.
  Evidence: dashboard / services/dashboard/src/main.ts @ e635069293d58cb0d4f28c8211e748b8d58e6b37 #L67-L74
- failures: Places returns 400 with error UNSUPPORTED_CITY when CityId maps to no city.
  Evidence: places / services/places/PlacesController.cs @ c289b28be4819eddf29d59a7c12f178559ec5229 #L10-L17
- failures: Dashboard maps an exception from fetch or response JSON parsing to 503 with PLACES_UNAVAILABLE; this handler does not reveal whether Places completed the save before that error.
  Evidence: dashboard / services/dashboard/src/main.ts @ e635069293d58cb0d4f28c8211e748b8d58e6b37 #L67-L74
- guarantees: The source shows no request idempotency key in these handlers; retry and duplicate-save semantics are unresolved at this boundary.
  Evidence: dashboard / services/dashboard/src/main.ts @ e635069293d58cb0d4f28c8211e748b8d58e6b37 #L67-L74
  Evidence: places / services/places/PlacesController.cs @ c289b28be4819eddf29d59a7c12f178559ec5229 #L10-L17
- guarantees: A successful Places response follows the awaited repository.Save call, but its storage and commit outcome are outside this wrapper contract.
  Evidence: places / services/places/PlacesController.cs @ c289b28be4819eddf29d59a7c12f178559ec5229 #L10-L17

[Receiving fragment places-save-handler](../services/places.html#service-fragment-dashboard-save-city:places-save-handler)

- **Forward POST /places**: Serialize request.body as JSON, send it with application/json and a 10-second abort signal, then forward the response status and parsed JSON. A caught fetch or JSON parsing error returns 503 PLACES_UNAVAILABLE; completion at Places before an error is unresolved. ([source](./dashboard-save-city.html#thread-source-dashboard-save-L67), lines 67–74).
  - [EXTERNAL_CALL] Send POST /places to the declared Places target. ([source](./dashboard-save-city.html#thread-source-dashboard-save-L69)).
  - **Handle the save request**: Map CityId. Return 400 for an unsupported city; otherwise await repository.Save and return the mapped city. The repository implementation and commit behavior are outside this controller wrapper. ([source](./dashboard-save-city.html#thread-source-places-save-L10), lines 10–17).
    - **Declared branches: IF_ELSE**
      - **city is null**
        - [guard source](./dashboard-save-city.html#thread-source-places-save-L14)
        - **Return unsupported-city response**: Return BadRequest with error UNSUPPORTED_CITY. ([source](./dashboard-save-city.html#thread-source-places-save-L14), lines 14–14).
          - [RETURN] Return HTTP 400 for an unsupported city. ([source](./dashboard-save-city.html#thread-source-places-save-L14)).
      - **Otherwise**
        - **Await the repository save**: Await repository.Save(city, token); this wrapper does not establish its storage or commit semantics. ([source](./dashboard-save-city.html#thread-source-places-save-L15), lines 15–15).
        - **Return the mapped city**: Return Ok(city) after the repository call returns. ([source](./dashboard-save-city.html#thread-source-places-save-L16), lines 16–16).
          - [RETURN] Return HTTP 200 with the mapped city. ([source](./dashboard-save-city.html#thread-source-places-save-L16)).

### Scope and boundaries

Dashboard.savePlace and the declared Places.Save HTTP boundary, with controller-visible request and response branches.

- The Dashboard-to-Places target is a declared HTTP boundary; compiler linkage and runtime routing are not established.
- The selected wrappers do not establish repository commit, timeout completion, retry, or duplicate-request behavior.

<details><summary>Selected thread sources</summary>

### Dashboard savePlace handler

dashboard · services/dashboard/src/main.ts:67–74 · e635069293d58cb0d4f28c8211e748b8d58e6b37

```
async function savePlace(request: FastifyRequest, reply: FastifyReply): Promise<void> {
  try {
    const response = await fetch(`${placesUrl}/places`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(request.body), signal: AbortSignal.timeout(10000) })
    reply.code(response.status).send(await response.json())
  } catch {
    reply.code(503).send({ error: 'PLACES_UNAVAILABLE' })
  }
}
```

### Places Save controller

places · services/places/PlacesController.cs:10–17 · c289b28be4819eddf29d59a7c12f178559ec5229

```
[HttpPost("/places")]
    public async Task<IActionResult> Save([FromBody] SavePlace request, CancellationToken token)
    {
        var city = PlacesRepository.City(request.CityId);
        if (city is null) return BadRequest(new { error = "UNSUPPORTED_CITY" });
        await repository.Save(city, token);
        return Ok(city);
    }
```

</details>
