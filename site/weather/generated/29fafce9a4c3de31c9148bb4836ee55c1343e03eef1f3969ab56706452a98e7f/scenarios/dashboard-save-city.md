# Dashboard save-city thread

Static source interpretation. Declared interactions do not establish runtime routing.

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

## dashboard-save-city

Documentation gap: Outside the requested work entrypoint.


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
