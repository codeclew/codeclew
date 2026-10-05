Read the complete native context and retained SOURCE parts below. Author only the responsibilities section using references from this Work. Explain byte-binding verification, presentation generation and the meaning/runtime limits. Preserve the existing overview. This section will remain unchanged in the maintenance exercise.

```json
{
  "audience": "An engineer maintaining the public source walkthrough",
  "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
  "contextDigest": "sha256:0ba04eb0eeff8d286e347bc9aac43b435960384cdb6ff9bf5d614bf1ee1904fc",
  "contextProfile": "section-orientation-v1",
  "documentationLanguage": "en",
  "influenceCoverage": "RECORDED_READS_ONLY_EXECUTION_NOT_ATTESTED",
  "inputDigest": "sha256:e13ce0de0ee03c8f090704a68d8156573bb59842ed406ea20463daec12ec54e2",
  "items": [
    {
      "documentationLanguageStatus": "NOT_AUTHORED",
      "id": "section-responsibilities",
      "kind": "SECTION",
      "record": {
        "content": null,
        "gap": "Required service section has not been authored in this proposal.",
        "id": "section-responsibilities",
        "objectId": "service:cli-documentation/section-responsibilities",
        "purpose": "Describe responsibilities and exclusions from evidence; business intent may need a human note.",
        "required": true,
        "schema": "codeclew-documentation-section/1.0",
        "service": "cli-documentation",
        "status": "GAP",
        "title": "Responsibilities",
        "workRequest": {
          "audience": "Service maintainers and architecture readers",
          "entrypoint": "section-responsibilities",
          "maxBytes": 40960,
          "maxItems": 20,
          "schema": "codeclew-documentation-work-request/1.0"
        }
      },
      "reference": "section2",
      "referenceRoles": [
        "operation",
        "gap"
      ],
      "requestedDocumentationLanguage": "en"
    },
    {
      "id": "inventory:cli-documentation",
      "kind": "BOUNDARY_INVENTORY",
      "record": {
        "gaps": [
          "PUBLIC_BOUNDARY_INVENTORY_IS_BOUNDED_BY_SELECTED_MODULES",
          "DYNAMIC_REGISTRATION_AND_RUNTIME_ACTIVATION_UNVERIFIED"
        ],
        "internalCallableCount": 5,
        "internalCallablePreviewLimits": {
          "maxBytes": 8192,
          "maxItems": 8
        },
        "internalCallables": [
          {
            "dependencyIds": [
              "cli-documentation:symbol:cd96038866bf7f18d601a02e"
            ],
            "id": "cli-documentation:symbol:cd96038866bf7f18d601a02e",
            "name": "diagram_sources",
            "owner": "package:scripts.build_cli_documentation",
            "parameterTypes": [
              "data"
            ],
            "publicBoundary": false,
            "scope": "",
            "service": "cli-documentation",
            "sourceIds": [
              "cli-documentation-cd96038866bf7f18d601"
            ],
            "status": "RECORDED",
            "symbol": "source:scripts/build_cli_documentation.py/package:scripts.build_cli_documentation/diagram_sources/1fcb8f233748fd46e02b"
          },
          {
            "dependencyIds": [
              "cli-documentation:symbol:dd6a072510e7af3faf22eb3a"
            ],
            "id": "cli-documentation:symbol:dd6a072510e7af3faf22eb3a",
            "name": "main",
            "owner": "package:scripts.build_cli_documentation",
            "parameterTypes": [],
            "publicBoundary": false,
            "scope": "",
            "service": "cli-documentation",
            "sourceIds": [
              "cli-documentation-dd6a072510e7af3faf22"
            ],
            "status": "RECORDED",
            "symbol": "source:scripts/build_cli_documentation.py/package:scripts.build_cli_documentation/main/f76638a0e73f8a97a3de"
          },
          {
            "dependencyIds": [
              "cli-documentation:symbol:de4cd73046421c440a6d3ef4"
            ],
            "id": "cli-documentation:symbol:de4cd73046421c440a6d3ef4",
            "name": "render_claims",
            "owner": "package:scripts.build_cli_documentation",
            "parameterTypes": [
              "claims"
            ],
            "publicBoundary": false,
            "scope": "",
            "service": "cli-documentation",
            "sourceIds": [
              "cli-documentation-de4cd73046421c440a6d"
            ],
            "status": "RECORDED",
            "symbol": "source:scripts/build_cli_documentation.py/package:scripts.build_cli_documentation/render_claims/bbe2af4e300709a41a10"
          },
          {
            "dependencyIds": [
              "cli-documentation:symbol:943e7c4c9b378cf363ac9936"
            ],
            "id": "cli-documentation:symbol:943e7c4c9b378cf363ac9936",
            "name": "render_svg",
            "owner": "package:scripts.build_cli_documentation",
            "parameterTypes": [
              "dot",
              "claims"
            ],
            "publicBoundary": false,
            "scope": "",
            "service": "cli-documentation",
            "sourceIds": [
              "cli-documentation-943e7c4c9b378cf363ac"
            ],
            "status": "RECORDED",
            "symbol": "source:scripts/build_cli_documentation.py/package:scripts.build_cli_documentation/render_svg/77befcebc84fae0ccf97"
          },
          {
            "dependencyIds": [
              "cli-documentation:symbol:30dea2dc5cb47627d15fe5c5"
            ],
            "id": "cli-documentation:symbol:30dea2dc5cb47627d15fe5c5",
            "name": "verify",
            "owner": "package:scripts.build_cli_documentation",
            "parameterTypes": [
              "data"
            ],
            "publicBoundary": false,
            "scope": "",
            "service": "cli-documentation",
            "sourceIds": [
              "cli-documentation-30dea2dc5cb47627d15f"
            ],
            "status": "RECORDED",
            "symbol": "source:scripts/build_cli_documentation.py/package:scripts.build_cli_documentation/verify/e9c1167d5de6c7160235"
          }
        ],
        "omittedInternalCallables": 0,
        "omittedPublicBoundaries": 0,
        "publicBoundaries": [],
        "publicBoundaryCount": 0,
        "sourceBoundaries": [
          "CALL_TARGETS_UNRESOLVED",
          "DIALECT_DECLARED_NOT_COMPILER_VALIDATED",
          "ORDER_LEXICAL_ONLY",
          "SCOPE_WATCH_CONSERVATIVE"
        ]
      },
      "referenceRoles": []
    },
    {
      "id": "cli-documentation:symbol:cd96038866bf7f18d601a02e",
      "kind": "DEPENDENCY",
      "record": {
        "digest": "sha256:218907dd1e9ed0436eece7afaea6d5fcdf3684dda25291500cf17aba6093ac56",
        "id": "cli-documentation:symbol:cd96038866bf7f18d601a02e",
        "kind": "SYMBOL",
        "normalized": {
          "authority": "SYNTAX",
          "documentation": {
            "boundaries": [
              "CALL_TARGETS_UNRESOLVED",
              "ORDER_LEXICAL_ONLY"
            ],
            "events": [
              {
                "authority": "SYNTAX",
                "fingerprint": "sha256:21f469e82907da0258889c5288c57aef01e5b2badaea66207c81342321493f9f",
                "kind": "LOOP",
                "nestingDepth": 1,
                "ordering": "LEXICAL_ONLY",
                "ordinal": 0,
                "parentOrdinal": null,
                "syntaxKind": "for_statement",
                "targetStatus": "UNRESOLVED",
                "text": "for node in data[\"diagram\"][\"nodes\"]:\n        id = node[\"id\"]\n        shape = \"diamond\" if id == \"decision\" else \"box\"\n        color = \"#e0ae65\" if id in {\"abstain\", \"failure\"} else \"#91b774\"\n        dot.append(f'{id} [label={quote(node[\"label\"])}, shape={shape}, color=\"{color}\", id=\"node-{id}\", URL=\"https://codeclew.github.io/codeclew/nav-query.html#claim-{id}\", tooltip={quote(id)}];')\n        label = node[\"label\"].replace(\"\\n\", \"<br/>\")\n        mermaid.append(f'  {id}[\"{label}\"]')"
              },
              {
                "authority": "SYNTAX",
                "fingerprint": "sha256:716667eff04f2931c1f8bd137992c9eaf47072c0b1b32385b58068ae1c7c90ac",
                "kind": "IF",
                "nestingDepth": 5,
                "ordering": "LEXICAL_ONLY",
                "ordinal": 1,
                "parentOrdinal": 0,
                "syntaxKind": "conditional_expression",
                "targetStatus": "UNRESOLVED",
                "text": "\"diamond\" if id == \"decision\" else \"box\""
              },
              {
                "authority": "SYNTAX",
                "fingerprint": "sha256:2be155c0027ebe154f83f7e458af804a019857b0149fb3e2741acbbaee2d43e1",
                "kind": "IF",
                "nestingDepth": 5,
                "ordering": "LEXICAL_ONLY",
                "ordinal": 2,
                "parentOrdinal": 0,
                "syntaxKind": "conditional_expression",
                "targetStatus": "UNRESOLVED",
                "text": "\"#e0ae65\" if id in {\"abstain\", \"failure\"} else \"#91b774\""
              },
              {
                "authority": "SYNTAX",
                "fingerprint": "sha256:5eef0adfbbe031894a6ffd459d976cc8203244c7371c37349ceaef9e5c40102b",
                "kind": "CALL",
                "nestingDepth": 4,
                "ordering": "LEXICAL_ONLY",
                "ordinal": 3,
                "parentOrdinal": 0,
                "syntaxKind": "call",
                "targetStatus": "UNRESOLVED",
                "text": "dot.append(f'{id} [label={quote(node[\"label\"])}, shape={shape}, color=\"{color}\", id=\"node-{id}\", URL=\"https://codeclew.github.io/codeclew/nav-query.html#claim-{id}\", tooltip={quote(id)}];')"
              },
              {
                "authority": "SYNTAX",
                "fingerprint": "sha256:09338ba12ec5ac03a299af76343fb7cf005fa5244e836ffc19cc4ff2d2698758",
                "kind": "CALL",
                "nestingDepth": 8,
                "ordering": "LEXICAL_ONLY",
                "ordinal": 4,
                "parentOrdinal": 3,
                "syntaxKind": "call",
                "targetStatus": "UNRESOLVED",
                "text": "quote(node[\"label\"])"
              },
              {
                "authority": "SYNTAX",
                "fingerprint": "sha256:d84a4cf11a2a40da395c9e01398dba90f57330853e330c59cfd4f95658f011d1",
                "kind": "CALL",
                "nestingDepth": 8,
                "ordering": "LEXICAL_ONLY",
                "ordinal": 5,
                "parentOrdinal": 3,
                "syntaxKind": "call",
                "targetStatus": "UNRESOLVED",
                "text": "quote(id)"
              },
              {
                "authority": "SYNTAX",
                "fingerprint": "sha256:354033df716a4031aba5571d74cef0acbc5dc188c642a7d8f6757eea983ecfd7",
                "kind": "CALL",
                "nestingDepth": 5,
                "ordering": "LEXICAL_ONLY",
                "ordinal": 6,
                "parentOrdinal": 0,
                "syntaxKind": "call",
                "targetStatus": "UNRESOLVED",
                "text": "node[\"label\"].replace(\"\\n\", \"<br/>\")"
              },
              {
                "authority": "SYNTAX",
                "fingerprint": "sha256:c8791f77b9e62f08173484c7ebb77a87a5f9073ba482aef9eb85f3cb5c6a2d71",
                "kind": "CALL",
                "nestingDepth": 4,
                "ordering": "LEXICAL_ONLY",
                "ordinal": 7,
                "parentOrdinal": 0,
                "syntaxKind": "call",
                "targetStatus": "UNRESOLVED",
                "text": "mermaid.append(f'  {id}[\"{label}\"]')"
              },
              {
                "authority": "SYNTAX",
                "fingerprint": "sha256:66ae9c2eea3052ac5e168539dc0feb7b5afa7378ef652e044ddda48863b92104",
                "kind": "CALL",
                "nestingDepth": 2,
                "ordering": "LEXICAL_ONLY",
                "ordinal": 8,
                "parentOrdinal": null,
                "syntaxKind": "call",
                "targetStatus": "UNRESOLVED",
                "text": "dot.append(\"{rank=same; supported; abstain;}\")"
              },
              {
                "authority": "SYNTAX",
                "fingerprint": "sha256:76d993f1839dfc599c491a50cc6e895e9215148a0d7434c69a0429f469e765a2",
                "kind": "LOOP",
                "nestingDepth": 1,
                "ordering": "LEXICAL_ONLY",
                "ordinal": 9,
                "parentOrdinal": null,
                "syntaxKind": "for_statement",
                "targetStatus": "UNRESOLVED",
                "text": "for index, edge in enumerate(data[\"diagram\"][\"edges\"]):\n        dot.append(f'{edge[\"from\"]} -> {edge[\"to\"]} [label={quote(edge[\"label\"])}, id=\"edge-{index}\", URL=\"https://codeclew.github.io/codeclew/nav-query.html#claim-{edge[\"claimId\"]}\", tooltip={quote(edge[\"authority\"])}];')\n        mermaid.append(f'  {edge[\"from\"]} -. \"{edge[\"label\"]} · claim:{edge[\"claimId\"]}\" .-> {edge[\"to\"]}')"
              },
              {
                "authority": "SYNTAX",
                "fingerprint": "sha256:eea538506b6db0c30821373af2d27ef90c83b3ca76d15960660eb322b1562fcb",
                "kind": "CALL",
                "nestingDepth": 2,
                "ordering": "LEXICAL_ONLY",
                "ordinal": 10,
                "parentOrdinal": 9,
                "syntaxKind": "call",
                "targetStatus": "UNRESOLVED",
                "text": "enumerate(data[\"diagram\"][\"edges\"])"
              },
              {
                "authority": "SYNTAX",
                "fingerprint": "sha256:bd1c9cfb5f93a3b3044ff270738c27cc2e28124659ee4eb85999dd607ab99641",
                "kind": "CALL",
                "nestingDepth": 4,
                "ordering": "LEXICAL_ONLY",
                "ordinal": 11,
                "parentOrdinal": 9,
                "syntaxKind": "call",
                "targetStatus": "UNRESOLVED",
                "text": "dot.append(f'{edge[\"from\"]} -> {edge[\"to\"]} [label={quote(edge[\"label\"])}, id=\"edge-{index}\", URL=\"https://codeclew.github.io/codeclew/nav-query.html#claim-{edge[\"claimId\"]}\", tooltip={quote(edge[\"authority\"])}];')"
              },
              {
                "authority": "SYNTAX",
                "fingerprint": "sha256:7dec969681bc79c493b5dd0cfb94c77e33e0e111520a41b0324824d70246f723",
                "kind": "CALL",
                "nestingDepth": 8,
                "ordering": "LEXICAL_ONLY",
                "ordinal": 12,
                "parentOrdinal": 11,
                "syntaxKind": "call",
                "targetStatus": "UNRESOLVED",
                "text": "quote(edge[\"label\"])"
              },
              {
                "authority": "SYNTAX",
                "fingerprint": "sha256:052322398a92e67cf91af09763d804be29022fbe7c6b391884c179a89ed08532",
                "kind": "CALL",
                "nestingDepth": 8,
                "ordering": "LEXICAL_ONLY",
                "ordinal": 13,
                "parentOrdinal": 11,
                "syntaxKind": "call",
                "targetStatus": "UNRESOLVED",
                "text": "quote(edge[\"authority\"])"
              },
              {
                "authority": "SYNTAX",
                "fingerprint": "sha256:51c2533a73eeaf19bbf87d3752ca7e9b0eec033e5edb40af6f38fe2f3eb7f0d4",
                "kind": "CALL",
                "nestingDepth": 4,
                "ordering": "LEXICAL_ONLY",
                "ordinal": 14,
                "parentOrdinal": 9,
                "syntaxKind": "call",
                "targetStatus": "UNRESOLVED",
                "text": "mermaid.append(f'  {edge[\"from\"]} -. \"{edge[\"label\"]} · claim:{edge[\"claimId\"]}\" .-> {edge[\"to\"]}')"
              },
              {
                "authority": "SYNTAX",
                "fingerprint": "sha256:77099117c51555a142ec266bee2d5accfd84598e60100c64fd76eed9b3d34665",
                "kind": "CALL",
                "nestingDepth": 2,
                "ordering": "LEXICAL_ONLY",
                "ordinal": 15,
                "parentOrdinal": null,
                "syntaxKind": "call",
                "targetStatus": "UNRESOLVED",
                "text": "dot.append(\"}\")"
              },
              {
                "authority": "SYNTAX",
                "fingerprint": "sha256:7727e9f577abbacae7d057f63e2a9dfa7e4fe223664f386edeb819b3fad70aa5",
                "kind": "RETURN",
                "nestingDepth": 1,
                "ordering": "LEXICAL_ONLY",
                "ordinal": 16,
                "parentOrdinal": null,
                "syntaxKind": "return_statement",
                "targetStatus": "UNRESOLVED",
                "text": "return \"\\n\".join(dot) + \"\\n\", \"\\n\".join(mermaid) + \"\\n\""
              },
              {
                "authority": "SYNTAX",
                "fingerprint": "sha256:030a98e5db8defea67d28b13423b7a2030a9ffc45b13f18cde5e83f043c274d1",
                "kind": "CALL",
                "nestingDepth": 4,
                "ordering": "LEXICAL_ONLY",
                "ordinal": 17,
                "parentOrdinal": 16,
                "syntaxKind": "call",
                "targetStatus": "UNRESOLVED",
                "text": "\"\\n\".join(dot)"
              },
              {
                "authority": "SYNTAX",
                "fingerprint": "sha256:c6d28f157d838d7bf0696c1086354aa6e4f29ccff42ea2fc2e4411aa7d420ca5",
                "kind": "CALL",
                "nestingDepth": 4,
                "ordering": "LEXICAL_ONLY",
                "ordinal": 18,
                "parentOrdinal": 16,
                "syntaxKind": "call",
                "targetStatus": "UNRESOLVED",
                "text": "\"\\n\".join(mermaid)"
              }
            ],
            "parameterTypes": [
              "data"
            ]
          },
          "fingerprint": "sha256:178736350dc97b19f5a3c21b172b3f2e261998d2a0f4f83218922cfc6f873a3e",
          "kind": "DECLARATION",
          "name": "diagram_sources",
          "ownerIdentity": "package:scripts.build_cli_documentation",
          "symbolIdentity": "source:scripts/build_cli_documentation.py/package:scripts.build_cli_documentation/diagram_sources/1fcb8f233748fd46e02b",
          "syntaxKind": "function_definition"
        },
        "service": "cli-documentation",
        "sourceIds": [
          "cli-documentation-cd96038866bf7f18d601"
        ],
        "symbol": "source:scripts/build_cli_documentation.py/package:scripts.build_cli_documentation/diagram_sources/1fcb8f233748fd46e02b"
      },
      "reference": "d120",
      "referenceRoles": [
        "evidence"
      ],
      "selectionReason": "discovered-entrypoint-declaration",
      "sourceReferences": [
        "s150"
      ]
    },
    {
      "id": "evidence-index:cli-documentation",
      "kind": "EVIDENCE_DISCOVERY",
      "record": {
        "availableDependencyCount": 183,
        "countsByKind": {
          "ENTITY_SCOPE": 1,
          "FLOW": 116,
          "NOTE_ASSOCIATION": 1,
          "NOTE_SCOPE": 1,
          "SOURCE_SCOPE": 1,
          "SYMBOL": 5,
          "SYNTAX_DETAIL": 58
        },
        "deferredDependencyCount": 182,
        "influenceCoverage": "Full captured Work influence is unchanged by this context preview.",
        "nextRead": "Use sourceReferences/dependencyReferences with work expand, or a query with kind and symbolContains. Deferred facts are not absent or unsupported; read them before citing them.",
        "previewLimits": {
          "maxBytes": 8192,
          "maxItems": 8
        },
        "section": "section-responsibilities",
        "selection": "Retained selected-section dependencies, then discovered entrypoint declarations. No arbitrary symbol sampling.",
        "suppliedDependencyCount": 1
      },
      "referenceRoles": []
    },
    {
      "id": "review-0",
      "kind": "REVIEW_REASON",
      "record": {
        "fragment": "service:cli-documentation/cli-documentation-4c4e5646fc3afe399640/entrypoint",
        "reasons": [
          {
            "after": "sha256:2632b450359a1428f22f6768eecca6e20aad8672c66f58bb2b7fbf6f504cc6a6",
            "before": "sha256:668a9516b3d5b3fd12ed418b7161a944ad13eb7312c3090b74c73fc4e27ce156",
            "dependency": "note-scope:cli-documentation",
            "reason": "SUPPORTED_BEHAVIOR_CHANGED"
          }
        ],
        "requiredAction": "REVIEW_AND_REGENERATE_FRAGMENT",
        "subject": "service:cli-documentation"
      },
      "referenceRoles": []
    },
    {
      "id": "review-1",
      "kind": "REVIEW_REASON",
      "record": {
        "fragment": "service:cli-documentation/cli-documentation-53b99d0f35fa55badac7/entrypoint",
        "reasons": [
          {
            "after": "sha256:2632b450359a1428f22f6768eecca6e20aad8672c66f58bb2b7fbf6f504cc6a6",
            "before": "sha256:668a9516b3d5b3fd12ed418b7161a944ad13eb7312c3090b74c73fc4e27ce156",
            "dependency": "note-scope:cli-documentation",
            "reason": "SUPPORTED_BEHAVIOR_CHANGED"
          }
        ],
        "requiredAction": "REVIEW_AND_REGENERATE_FRAGMENT",
        "subject": "service:cli-documentation"
      },
      "referenceRoles": []
    },
    {
      "id": "review-2",
      "kind": "REVIEW_REASON",
      "record": {
        "fragment": "service:cli-documentation/cli-documentation-672ee5015ada81ec8d40/entrypoint",
        "reasons": [
          {
            "after": "sha256:2632b450359a1428f22f6768eecca6e20aad8672c66f58bb2b7fbf6f504cc6a6",
            "before": "sha256:668a9516b3d5b3fd12ed418b7161a944ad13eb7312c3090b74c73fc4e27ce156",
            "dependency": "note-scope:cli-documentation",
            "reason": "SUPPORTED_BEHAVIOR_CHANGED"
          }
        ],
        "requiredAction": "REVIEW_AND_REGENERATE_FRAGMENT",
        "subject": "service:cli-documentation"
      },
      "referenceRoles": []
    },
    {
      "id": "review-3",
      "kind": "REVIEW_REASON",
      "record": {
        "fragment": "service:cli-documentation/cli-documentation-6d1878f45ec2b985c9a6/entrypoint",
        "reasons": [
          {
            "after": "sha256:2632b450359a1428f22f6768eecca6e20aad8672c66f58bb2b7fbf6f504cc6a6",
            "before": "sha256:668a9516b3d5b3fd12ed418b7161a944ad13eb7312c3090b74c73fc4e27ce156",
            "dependency": "note-scope:cli-documentation",
            "reason": "SUPPORTED_BEHAVIOR_CHANGED"
          }
        ],
        "requiredAction": "REVIEW_AND_REGENERATE_FRAGMENT",
        "subject": "service:cli-documentation"
      },
      "referenceRoles": []
    },
    {
      "id": "review-4",
      "kind": "REVIEW_REASON",
      "record": {
        "fragment": "service:cli-documentation/cli-documentation-7368fc134a99e3cdace1/entrypoint",
        "reasons": [
          {
            "after": "sha256:2632b450359a1428f22f6768eecca6e20aad8672c66f58bb2b7fbf6f504cc6a6",
            "before": "sha256:668a9516b3d5b3fd12ed418b7161a944ad13eb7312c3090b74c73fc4e27ce156",
            "dependency": "note-scope:cli-documentation",
            "reason": "SUPPORTED_BEHAVIOR_CHANGED"
          }
        ],
        "requiredAction": "REVIEW_AND_REGENERATE_FRAGMENT",
        "subject": "service:cli-documentation"
      },
      "referenceRoles": []
    },
    {
      "id": "review-5",
      "kind": "REVIEW_REASON",
      "record": {
        "fragment": "service:cli-documentation/entity-catalogue",
        "reasons": [
          {
            "after": "sha256:2632b450359a1428f22f6768eecca6e20aad8672c66f58bb2b7fbf6f504cc6a6",
            "before": "sha256:668a9516b3d5b3fd12ed418b7161a944ad13eb7312c3090b74c73fc4e27ce156",
            "dependency": "note-scope:cli-documentation",
            "reason": "SUPPORTED_BEHAVIOR_CHANGED"
          }
        ],
        "requiredAction": "REVIEW_AND_REGENERATE_FRAGMENT",
        "subject": "service:cli-documentation"
      },
      "referenceRoles": []
    },
    {
      "id": "review-6",
      "kind": "REVIEW_REASON",
      "record": {
        "fragment": "service:cli-documentation/note-catalogue",
        "reasons": [
          {
            "after": "sha256:2632b450359a1428f22f6768eecca6e20aad8672c66f58bb2b7fbf6f504cc6a6",
            "before": "sha256:668a9516b3d5b3fd12ed418b7161a944ad13eb7312c3090b74c73fc4e27ce156",
            "dependency": "note-scope:cli-documentation",
            "reason": "SUPPORTED_BEHAVIOR_CHANGED"
          }
        ],
        "requiredAction": "REVIEW_AND_REGENERATE_FRAGMENT",
        "subject": "service:cli-documentation"
      },
      "referenceRoles": []
    },
    {
      "id": "review-7",
      "kind": "REVIEW_REASON",
      "record": {
        "fragment": "service:cli-documentation/section-overview/claim-55430bd5cdb903284513",
        "reasons": [
          {
            "after": "sha256:2632b450359a1428f22f6768eecca6e20aad8672c66f58bb2b7fbf6f504cc6a6",
            "before": "sha256:668a9516b3d5b3fd12ed418b7161a944ad13eb7312c3090b74c73fc4e27ce156",
            "dependency": "note-scope:cli-documentation",
            "reason": "SUPPORTED_BEHAVIOR_CHANGED"
          },
          {
            "reason": "INFLUENCE_SCOPE_CHANGED",
            "scope": "sha256:59b613bfa84b6ee6ed234a97b1696c0a215c8518df5d08c843dbd74a49d51d56"
          }
        ],
        "requiredAction": "REVIEW_AND_REGENERATE_FRAGMENT",
        "subject": "service:cli-documentation"
      },
      "referenceRoles": []
    },
    {
      "id": "review-8",
      "kind": "REVIEW_REASON",
      "record": {
        "fragment": "service:cli-documentation/section-overview/title",
        "reasons": [
          {
            "after": "sha256:2632b450359a1428f22f6768eecca6e20aad8672c66f58bb2b7fbf6f504cc6a6",
            "before": "sha256:668a9516b3d5b3fd12ed418b7161a944ad13eb7312c3090b74c73fc4e27ce156",
            "dependency": "note-scope:cli-documentation",
            "reason": "SUPPORTED_BEHAVIOR_CHANGED"
          },
          {
            "reason": "INFLUENCE_SCOPE_CHANGED",
            "scope": "sha256:59b613bfa84b6ee6ed234a97b1696c0a215c8518df5d08c843dbd74a49d51d56"
          }
        ],
        "requiredAction": "REVIEW_AND_REGENERATE_FRAGMENT",
        "subject": "service:cli-documentation"
      },
      "referenceRoles": []
    },
    {
      "id": "review-9",
      "kind": "REVIEW_REASON",
      "record": {
        "fragment": "service:cli-documentation/source-scope",
        "reasons": [
          {
            "after": "sha256:2632b450359a1428f22f6768eecca6e20aad8672c66f58bb2b7fbf6f504cc6a6",
            "before": "sha256:668a9516b3d5b3fd12ed418b7161a944ad13eb7312c3090b74c73fc4e27ce156",
            "dependency": "note-scope:cli-documentation",
            "reason": "SUPPORTED_BEHAVIOR_CHANGED"
          }
        ],
        "requiredAction": "REVIEW_AND_REGENERATE_FRAGMENT",
        "subject": "service:cli-documentation"
      },
      "referenceRoles": []
    },
    {
      "id": "notes/walkthrough-owner-guidance.md",
      "kind": "EXTERNAL_INPUT",
      "record": {
        "authority": "HUMAN_OR_IMPORTED_UNVERIFIED",
        "digest": "sha256:e6715808230e8eff2c7c5cca7be919ddca02bf33fcba7a2ed0b5748e9c1e5b1c",
        "status": "CAPTURED",
        "text": "# Owner guidance for this walkthrough\n\nPreserve the byte-binding verification explanation and the responsibilities section when updating the overview. Label the follow-up SVG-root validation as a caller-owned local maintenance exercise, not a feature of the released Codeclew source. Runtime behavior and semantic correctness of agent-inferred claims remain outside this note.\n"
      },
      "referenceRoles": []
    },
    {
      "id": "obligation-1",
      "kind": "OBLIGATION",
      "record": {
        "detail": "CALL_TARGETS_UNRESOLVED",
        "kind": "EVIDENCE_BOUNDARY",
        "service": "cli-documentation"
      },
      "referenceRoles": []
    },
    {
      "id": "obligation-2",
      "kind": "OBLIGATION",
      "record": {
        "detail": "DIALECT_DECLARED_NOT_COMPILER_VALIDATED",
        "kind": "EVIDENCE_BOUNDARY",
        "service": "cli-documentation"
      },
      "referenceRoles": []
    },
    {
      "id": "obligation-3",
      "kind": "OBLIGATION",
      "record": {
        "detail": "ORDER_LEXICAL_ONLY",
        "kind": "EVIDENCE_BOUNDARY",
        "service": "cli-documentation"
      },
      "referenceRoles": []
    },
    {
      "id": "obligation-4",
      "kind": "OBLIGATION",
      "record": {
        "detail": "SCOPE_WATCH_CONSERVATIVE",
        "kind": "EVIDENCE_BOUNDARY",
        "service": "cli-documentation"
      },
      "referenceRoles": []
    },
    {
      "id": "obligation-5",
      "kind": "OBLIGATION",
      "record": {
        "detail": "Syntax evidence does not establish runtime dispatch or configuration. Expand known owners and helpers; retain explicit gaps for unresolved targets.",
        "kind": "UNRESOLVED_CALL_AUTHORITY",
        "service": "cli-documentation"
      },
      "referenceRoles": []
    }
  ],
  "membershipDigest": "sha256:c679484f323f748e93d61524a65e77976d92cc230498ff48bc84264ac3e8d404",
  "nextCursor": null,
  "omitted": [],
  "receiptDigest": "sha256:21f101e4681e350144e745990a78293b01bd24adb96407bedb4301a6a9716acc",
  "schema": "codeclew-documentation-work-page/1.0",
  "snapshot": "sha256:12d7a16bc6dbb7e9c522d3813b06dc095164a5d959c92b46d6cf39958b8e532b/4489",
  "subject": "service:cli-documentation",
  "total": 20,
  "work": "530cc649966e8655e58362ea7c779243626fb50977500f92b610738cca8aa724"
}
```

```json
{
  "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
  "endByte": 1329,
  "fragmentDigest": "sha256:d530422d767f53f73654d17b5ef3caf82c0221314dc8d07faf74aedc7f62d928",
  "nextCursor": null,
  "receiptDigest": "sha256:dce402dfdaa562eb23b458b3dabb30ae5088c6b4e677296068cb9617b06cdc1f",
  "receiptType": "SOURCE_PART",
  "recordDigest": "sha256:6e96ee7ff74dad1c9d3a6c31b674fd065e172360ac20a2ee064bd4185ff08963",
  "reference": "s109",
  "schema": "codeclew-documentation-source-part/1.0",
  "snapshot": "sha256:12d7a16bc6dbb7e9c522d3813b06dc095164a5d959c92b46d6cf39958b8e532b/4489",
  "source": {
    "authority": "EXACT_SNAPSHOT_TEXT",
    "endLine": 106,
    "evidenceDigest": "sha256:8977f3a69828b673d5b72d34f37417715d5a8905022080765c1ac79c0372638c",
    "file": "scripts/build_cli_documentation.py",
    "id": "cli-documentation-943e7c4c9b378cf363ac",
    "occurrence": {
      "blob": "a6352ae84b6909b7ab2aacdcf964bf679c1dff78",
      "endByte": 5523,
      "snapshot": "sha256:c315b611c3a5c118f97f5c2d40023779b01e34138cac6334cc086550156a9262",
      "startByte": 4194
    },
    "revision": "d91dbec1164e0601d47c221ab97c504033ce858e",
    "service": "cli-documentation",
    "startLine": 84,
    "textDigest": "sha256:d530422d767f53f73654d17b5ef3caf82c0221314dc8d07faf74aedc7f62d928",
    "url": "https://github.com/codeclew/codeclew/blob/d91dbec1164e0601d47c221ab97c504033ce858e/scripts/build_cli_documentation.py#L84-L106"
  },
  "sourceId": "cli-documentation-943e7c4c9b378cf363ac",
  "startByte": 0,
  "text": "def render_svg(dot, claims):\n    raw = subprocess.check_output([\"dot\", \"-Tsvg\"], input=dot.encode())\n    ET.register_namespace(\"\", \"http://www.w3.org/2000/svg\")\n    ET.register_namespace(\"xlink\", \"http://www.w3.org/1999/xlink\")\n    svg = ET.fromstring(raw)\n    svg.attrib.pop(\"width\", None)\n    svg.attrib.pop(\"height\", None)\n    svg.set(\"aria-labelledby\", \"flow-title flow-description\")\n    ns = \"{http://www.w3.org/2000/svg}\"\n    title = ET.Element(ns + \"title\", id=\"flow-title\")\n    title.text = \"How nav query moves from a request to bounded source evidence\"\n    description = ET.Element(ns + \"desc\", id=\"flow-description\")\n    description.text = \"Select a step or arrow to inspect its supporting code. Task readiness precedes context creation. The decision branches into SUPPORTED or ABSTAIN. Errors leave the main path. Dashed arrows are agent-interpreted static flow, not runtime observations.\"\n    svg.insert(0, description)\n    svg.insert(0, title)\n    for link in svg.iter(ns + \"a\"):\n        href = link.get(\"{http://www.w3.org/1999/xlink}href\", \"\")\n        id = href.rsplit(\"#claim-\", 1)[-1]\n        assert id in claims\n        link.set(\"data-claim\", id)\n        link.set(\"aria-label\", claims[id][\"title\"] + \": inspect source evidence\")\n        link.set(\"tabindex\", \"0\")\n    return ET.tostring(svg, encoding=\"unicode\")",
  "totalTextBytes": 1329,
  "work": "530cc649966e8655e58362ea7c779243626fb50977500f92b610738cca8aa724"
}
```

```json
{
  "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
  "endByte": 34,
  "fragmentDigest": "sha256:78da68f04770f73f278d71993cf63b8c6cdd241a187ea2e3cee16fa9f9f02535",
  "nextCursor": null,
  "receiptDigest": "sha256:4b6145971cad1b9c5ce57458177c1728719d0219c71e8ffff0ea3fd238acf045",
  "receiptType": "SOURCE_PART",
  "recordDigest": "sha256:b11d6f0018703115a75f5acd005ff1c22dd29c2929c52bf59e0be7e6b03d3f23",
  "reference": "s126",
  "schema": "codeclew-documentation-source-part/1.0",
  "snapshot": "sha256:12d7a16bc6dbb7e9c522d3813b06dc095164a5d959c92b46d6cf39958b8e532b/4489",
  "source": {
    "authority": "EXACT_SNAPSHOT_TEXT",
    "endLine": 20,
    "evidenceDigest": "sha256:49d2e24fbacabb4a999c6010f4dc43653b7b8986ff48279d3b494245b03b779a",
    "file": "scripts/build_cli_documentation.py",
    "id": "cli-documentation-acffade18fc7b97b24fb",
    "occurrence": {
      "blob": "a6352ae84b6909b7ab2aacdcf964bf679c1dff78",
      "endByte": 612,
      "snapshot": "sha256:c315b611c3a5c118f97f5c2d40023779b01e34138cac6334cc086550156a9262",
      "startByte": 578
    },
    "revision": "d91dbec1164e0601d47c221ab97c504033ce858e",
    "service": "cli-documentation",
    "startLine": 20,
    "textDigest": "sha256:78da68f04770f73f278d71993cf63b8c6cdd241a187ea2e3cee16fa9f9f02535",
    "url": "https://github.com/codeclew/codeclew/blob/d91dbec1164e0601d47c221ab97c504033ce858e/scripts/build_cli_documentation.py#L20-L20"
  },
  "sourceId": "cli-documentation-acffade18fc7b97b24fb",
  "startByte": 0,
  "text": "START = \"<!-- NAV_QUERY_GRAPH -->\"",
  "totalTextBytes": 34,
  "work": "530cc649966e8655e58362ea7c779243626fb50977500f92b610738cca8aa724"
}
```

```json
{
  "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
  "endByte": 24,
  "fragmentDigest": "sha256:6d69f2ac09017b8a49d050ab336e953b8fa02ae08c24396916ea7ea42418e627",
  "nextCursor": null,
  "receiptDigest": "sha256:08c41602a6861b3e2f834e3cb232b6a674574e951cd0027af532e07c3ba248ec",
  "receiptType": "SOURCE_PART",
  "recordDigest": "sha256:9889ab30dd9bbfbeadebd2487bd9c71578b8cfdbc7f49a66bc63108a2b6bed1c",
  "reference": "s131",
  "schema": "codeclew-documentation-source-part/1.0",
  "snapshot": "sha256:12d7a16bc6dbb7e9c522d3813b06dc095164a5d959c92b46d6cf39958b8e532b/4489",
  "source": {
    "authority": "EXACT_SNAPSHOT_TEXT",
    "endLine": 13,
    "evidenceDigest": "sha256:100e7290ba3e761be0574896673c187e9c08049047e4a8b7a3fb24d3fdb00c83",
    "file": "scripts/build_cli_documentation.py",
    "id": "cli-documentation-ba33901b52781d65787d",
    "occurrence": {
      "blob": "a6352ae84b6909b7ab2aacdcf964bf679c1dff78",
      "endByte": 425,
      "snapshot": "sha256:c315b611c3a5c118f97f5c2d40023779b01e34138cac6334cc086550156a9262",
      "startByte": 401
    },
    "revision": "d91dbec1164e0601d47c221ab97c504033ce858e",
    "service": "cli-documentation",
    "startLine": 13,
    "textDigest": "sha256:6d69f2ac09017b8a49d050ab336e953b8fa02ae08c24396916ea7ea42418e627",
    "url": "https://github.com/codeclew/codeclew/blob/d91dbec1164e0601d47c221ab97c504033ce858e/scripts/build_cli_documentation.py#L13-L13"
  },
  "sourceId": "cli-documentation-ba33901b52781d65787d",
  "startByte": 0,
  "text": "from pathlib import Path",
  "totalTextBytes": 24,
  "work": "530cc649966e8655e58362ea7c779243626fb50977500f92b610738cca8aa724"
}
```

```json
{
  "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
  "endByte": 11,
  "fragmentDigest": "sha256:6a1da67899bb57ec2903c3f99953fe8beb48d7cdb0bcad6e6da6c0c01bec04da",
  "nextCursor": null,
  "receiptDigest": "sha256:92de267b102303176dfa69bbc36b529137e09f55e1f2cc6c042321087f8c2fb6",
  "receiptType": "SOURCE_PART",
  "recordDigest": "sha256:56671f2f651301ee4bfdddf6ef291dacce8941d54bfa09e374a4e9969ff52ba3",
  "reference": "s139",
  "schema": "codeclew-documentation-source-part/1.0",
  "snapshot": "sha256:12d7a16bc6dbb7e9c522d3813b06dc095164a5d959c92b46d6cf39958b8e532b/4489",
  "source": {
    "authority": "EXACT_SNAPSHOT_TEXT",
    "endLine": 11,
    "evidenceDigest": "sha256:76a7bcdd12cbf83d7b7291998755db5298db6fda18be4990d54c7fa1c3460a7d",
    "file": "scripts/build_cli_documentation.py",
    "id": "cli-documentation-c1fe435bbd111dc55274",
    "occurrence": {
      "blob": "a6352ae84b6909b7ab2aacdcf964bf679c1dff78",
      "endByte": 388,
      "snapshot": "sha256:c315b611c3a5c118f97f5c2d40023779b01e34138cac6334cc086550156a9262",
      "startByte": 377
    },
    "revision": "d91dbec1164e0601d47c221ab97c504033ce858e",
    "service": "cli-documentation",
    "startLine": 11,
    "textDigest": "sha256:6a1da67899bb57ec2903c3f99953fe8beb48d7cdb0bcad6e6da6c0c01bec04da",
    "url": "https://github.com/codeclew/codeclew/blob/d91dbec1164e0601d47c221ab97c504033ce858e/scripts/build_cli_documentation.py#L11-L11"
  },
  "sourceId": "cli-documentation-c1fe435bbd111dc55274",
  "startByte": 0,
  "text": "import html",
  "totalTextBytes": 11,
  "work": "530cc649966e8655e58362ea7c779243626fb50977500f92b610738cca8aa724"
}
```

```json
{
  "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
  "endByte": 33,
  "fragmentDigest": "sha256:109b2e8cf37fd749893d0f117436a4cb08bc88f1b667868eb2a1f2f4f32ff759",
  "nextCursor": null,
  "receiptDigest": "sha256:d8157db7b5c18230e471a9d30d5d4f4e4cbf73a648ba02710148d0059407b1be",
  "receiptType": "SOURCE_PART",
  "recordDigest": "sha256:75c2535b6bef50ccd584844dd9f3dfd77d5c59fe8d7681b67f99a4e364d19306",
  "reference": "s141",
  "schema": "codeclew-documentation-source-part/1.0",
  "snapshot": "sha256:12d7a16bc6dbb7e9c522d3813b06dc095164a5d959c92b46d6cf39958b8e532b/4489",
  "source": {
    "authority": "EXACT_SNAPSHOT_TEXT",
    "endLine": 21,
    "evidenceDigest": "sha256:44979b9504d99cc21e26526a2932fbb4fab595709e2f71825039d4e0d4f12a98",
    "file": "scripts/build_cli_documentation.py",
    "id": "cli-documentation-c4e7efaf1c525edeadc2",
    "occurrence": {
      "blob": "a6352ae84b6909b7ab2aacdcf964bf679c1dff78",
      "endByte": 646,
      "snapshot": "sha256:c315b611c3a5c118f97f5c2d40023779b01e34138cac6334cc086550156a9262",
      "startByte": 613
    },
    "revision": "d91dbec1164e0601d47c221ab97c504033ce858e",
    "service": "cli-documentation",
    "startLine": 21,
    "textDigest": "sha256:109b2e8cf37fd749893d0f117436a4cb08bc88f1b667868eb2a1f2f4f32ff759",
    "url": "https://github.com/codeclew/codeclew/blob/d91dbec1164e0601d47c221ab97c504033ce858e/scripts/build_cli_documentation.py#L21-L21"
  },
  "sourceId": "cli-documentation-c4e7efaf1c525edeadc2",
  "startByte": 0,
  "text": "END = \"<!-- /NAV_QUERY_GRAPH -->\"",
  "totalTextBytes": 33,
  "work": "530cc649966e8655e58362ea7c779243626fb50977500f92b610738cca8aa724"
}
```

```json
{
  "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
  "endByte": 9,
  "fragmentDigest": "sha256:66ec018201fb03f10df19f1a56a1264beb2654e8f07c2f163f023ebd53a751e4",
  "nextCursor": null,
  "receiptDigest": "sha256:cba63db834ec23545a72e9d57c11db7295226b80a357540c4341de611a832cc0",
  "receiptType": "SOURCE_PART",
  "recordDigest": "sha256:e18bbde2d9f49ac0f3e47a832d94935f935e10e043d081e9d4c782d62df13f4b",
  "reference": "s143",
  "schema": "codeclew-documentation-source-part/1.0",
  "snapshot": "sha256:12d7a16bc6dbb7e9c522d3813b06dc095164a5d959c92b46d6cf39958b8e532b/4489",
  "source": {
    "authority": "EXACT_SNAPSHOT_TEXT",
    "endLine": 14,
    "evidenceDigest": "sha256:3c604d8ebc7b9117fc3caf34ed48616296cdeb0c94caad3c61b807f961c789e5",
    "file": "scripts/build_cli_documentation.py",
    "id": "cli-documentation-c666421815f60a3267ee",
    "occurrence": {
      "blob": "a6352ae84b6909b7ab2aacdcf964bf679c1dff78",
      "endByte": 435,
      "snapshot": "sha256:c315b611c3a5c118f97f5c2d40023779b01e34138cac6334cc086550156a9262",
      "startByte": 426
    },
    "revision": "d91dbec1164e0601d47c221ab97c504033ce858e",
    "service": "cli-documentation",
    "startLine": 14,
    "textDigest": "sha256:66ec018201fb03f10df19f1a56a1264beb2654e8f07c2f163f023ebd53a751e4",
    "url": "https://github.com/codeclew/codeclew/blob/d91dbec1164e0601d47c221ab97c504033ce858e/scripts/build_cli_documentation.py#L14-L14"
  },
  "sourceId": "cli-documentation-c666421815f60a3267ee",
  "startByte": 0,
  "text": "import re",
  "totalTextBytes": 9,
  "work": "530cc649966e8655e58362ea7c779243626fb50977500f92b610738cca8aa724"
}
```

```json
{
  "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
  "endByte": 34,
  "fragmentDigest": "sha256:925185a804f421190a7a0276a73594730bf5ea098b8fd7fb781ad42e365b8cc9",
  "nextCursor": null,
  "receiptDigest": "sha256:f2996ebe374d7dc9c33abaab7874a4dd6ee0a73deaad99732d8fdf5f1e1cbf8f",
  "receiptType": "SOURCE_PART",
  "recordDigest": "sha256:023b4256859334062b8962f8dd4fcdbc1f0f6bccc4f1cdffd8ba67dff02d883b",
  "reference": "s15",
  "schema": "codeclew-documentation-source-part/1.0",
  "snapshot": "sha256:12d7a16bc6dbb7e9c522d3813b06dc095164a5d959c92b46d6cf39958b8e532b/4489",
  "source": {
    "authority": "EXACT_SNAPSHOT_TEXT",
    "endLine": 16,
    "evidenceDigest": "sha256:f4d161edbe1b132d9995e691f574904906a997dd5ada23cef032a99c83e64ccb",
    "file": "scripts/build_cli_documentation.py",
    "id": "cli-documentation-104d704bda667a48addc",
    "occurrence": {
      "blob": "a6352ae84b6909b7ab2aacdcf964bf679c1dff78",
      "endByte": 488,
      "snapshot": "sha256:c315b611c3a5c118f97f5c2d40023779b01e34138cac6334cc086550156a9262",
      "startByte": 454
    },
    "revision": "d91dbec1164e0601d47c221ab97c504033ce858e",
    "service": "cli-documentation",
    "startLine": 16,
    "textDigest": "sha256:925185a804f421190a7a0276a73594730bf5ea098b8fd7fb781ad42e365b8cc9",
    "url": "https://github.com/codeclew/codeclew/blob/d91dbec1164e0601d47c221ab97c504033ce858e/scripts/build_cli_documentation.py#L16-L16"
  },
  "sourceId": "cli-documentation-104d704bda667a48addc",
  "startByte": 0,
  "text": "import xml.etree.ElementTree as ET",
  "totalTextBytes": 34,
  "work": "530cc649966e8655e58362ea7c779243626fb50977500f92b610738cca8aa724"
}
```

```json
{
  "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
  "endByte": 1587,
  "fragmentDigest": "sha256:89d954721b595b0a91558e6660a5854054f8e84569c7024fefd5cb463a2d5b7c",
  "nextCursor": null,
  "receiptDigest": "sha256:17a27adbec8957e2daf0d20fb1d6379eaaf38c9546005041df8c31474aba76c1",
  "receiptType": "SOURCE_PART",
  "recordDigest": "sha256:56155707f9aeb24339a155dc5af9dcf98c9c08579d97ce0c84dea3700a570d9c",
  "reference": "s150",
  "schema": "codeclew-documentation-source-part/1.0",
  "snapshot": "sha256:12d7a16bc6dbb7e9c522d3813b06dc095164a5d959c92b46d6cf39958b8e532b/4489",
  "source": {
    "authority": "EXACT_SNAPSHOT_TEXT",
    "endLine": 81,
    "evidenceDigest": "sha256:0e9f6f6375c71553d5258d55f12fae8a24a1038d1ee0235da992ea7366f308ff",
    "file": "scripts/build_cli_documentation.py",
    "id": "cli-documentation-cd96038866bf7f18d601",
    "occurrence": {
      "blob": "a6352ae84b6909b7ab2aacdcf964bf679c1dff78",
      "endByte": 4191,
      "snapshot": "sha256:c315b611c3a5c118f97f5c2d40023779b01e34138cac6334cc086550156a9262",
      "startByte": 2604
    },
    "revision": "d91dbec1164e0601d47c221ab97c504033ce858e",
    "service": "cli-documentation",
    "startLine": 60,
    "textDigest": "sha256:89d954721b595b0a91558e6660a5854054f8e84569c7024fefd5cb463a2d5b7c",
    "url": "https://github.com/codeclew/codeclew/blob/d91dbec1164e0601d47c221ab97c504033ce858e/scripts/build_cli_documentation.py#L60-L81"
  },
  "sourceId": "cli-documentation-cd96038866bf7f18d601",
  "startByte": 0,
  "text": "def diagram_sources(data):\n    quote = json.dumps\n    dot = [\n        \"digraph nav_query {\",\n        'graph [bgcolor=\"transparent\", rankdir=TB, pad=\"0.3\", nodesep=\"0.4\", ranksep=\"0.48\"];',\n        'node [shape=box, style=\"rounded,filled\", fillcolor=\"#151b16\", color=\"#52654b\", fontcolor=\"#f2f4ef\", fontname=\"Arial\", fontsize=15, margin=\"0.2,0.16\"];',\n        'edge [color=\"#77876c\", fontcolor=\"#bdc8b6\", fontname=\"Arial\", fontsize=11, arrowsize=0.65, style=dashed];',\n    ]\n    mermaid = [\"flowchart TD\", \"  %% Agent-interpreted static flow; no resolved Rust call graph.\"]\n    for node in data[\"diagram\"][\"nodes\"]:\n        id = node[\"id\"]\n        shape = \"diamond\" if id == \"decision\" else \"box\"\n        color = \"#e0ae65\" if id in {\"abstain\", \"failure\"} else \"#91b774\"\n        dot.append(f'{id} [label={quote(node[\"label\"])}, shape={shape}, color=\"{color}\", id=\"node-{id}\", URL=\"https://codeclew.github.io/codeclew/nav-query.html#claim-{id}\", tooltip={quote(id)}];')\n        label = node[\"label\"].replace(\"\\n\", \"<br/>\")\n        mermaid.append(f'  {id}[\"{label}\"]')\n    dot.append(\"{rank=same; supported; abstain;}\")\n    for index, edge in enumerate(data[\"diagram\"][\"edges\"]):\n        dot.append(f'{edge[\"from\"]} -> {edge[\"to\"]} [label={quote(edge[\"label\"])}, id=\"edge-{index}\", URL=\"https://codeclew.github.io/codeclew/nav-query.html#claim-{edge[\"claimId\"]}\", tooltip={quote(edge[\"authority\"])}];')\n        mermaid.append(f'  {edge[\"from\"]} -. \"{edge[\"label\"]} · claim:{edge[\"claimId\"]}\" .-> {edge[\"to\"]}')\n    dot.append(\"}\")\n    return \"\\n\".join(dot) + \"\\n\", \"\\n\".join(mermaid) + \"\\n\"",
  "totalTextBytes": 1587,
  "work": "530cc649966e8655e58362ea7c779243626fb50977500f92b610738cca8aa724"
}
```

```json
{
  "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
  "endByte": 14,
  "fragmentDigest": "sha256:ada1d9e0a360cda0fd1c8757d297aa74a4fc8d7193a69d3c28b43afc6950e9f9",
  "nextCursor": null,
  "receiptDigest": "sha256:2511616eb6d66306f190656a90c8861c0ad584a05499e510219c4376a7e0b821",
  "receiptType": "SOURCE_PART",
  "recordDigest": "sha256:692b2e23ae2d2d7f9509c7f03cfb39293a5ff0d65aff553e59a353df6243ebb1",
  "reference": "s158",
  "schema": "codeclew-documentation-source-part/1.0",
  "snapshot": "sha256:12d7a16bc6dbb7e9c522d3813b06dc095164a5d959c92b46d6cf39958b8e532b/4489",
  "source": {
    "authority": "EXACT_SNAPSHOT_TEXT",
    "endLine": 10,
    "evidenceDigest": "sha256:7f91aedb1683bd99e981b0a5bc90016c2baa3f56af580662b9a94a8face5b46e",
    "file": "scripts/build_cli_documentation.py",
    "id": "cli-documentation-dc0175ac0c2bda658fa5",
    "occurrence": {
      "blob": "a6352ae84b6909b7ab2aacdcf964bf679c1dff78",
      "endByte": 376,
      "snapshot": "sha256:c315b611c3a5c118f97f5c2d40023779b01e34138cac6334cc086550156a9262",
      "startByte": 362
    },
    "revision": "d91dbec1164e0601d47c221ab97c504033ce858e",
    "service": "cli-documentation",
    "startLine": 10,
    "textDigest": "sha256:ada1d9e0a360cda0fd1c8757d297aa74a4fc8d7193a69d3c28b43afc6950e9f9",
    "url": "https://github.com/codeclew/codeclew/blob/d91dbec1164e0601d47c221ab97c504033ce858e/scripts/build_cli_documentation.py#L10-L10"
  },
  "sourceId": "cli-documentation-dc0175ac0c2bda658fa5",
  "startByte": 0,
  "text": "import hashlib",
  "totalTextBytes": 14,
  "work": "530cc649966e8655e58362ea7c779243626fb50977500f92b610738cca8aa724"
}
```

```json
{
  "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
  "endByte": 1646,
  "fragmentDigest": "sha256:42b270da255c30e3034a074250236d483ed836d285997214e99d40cd078976cf",
  "nextCursor": null,
  "receiptDigest": "sha256:2219d159ad519c0401ac9c7af714b78700db64cc3272f1b97e7d653a8c94f7c9",
  "receiptType": "SOURCE_PART",
  "recordDigest": "sha256:1abcd4bed4b55ce6bd0e4913bb78e1f1e75d71d461fc6b72a4f0afadcaf10810",
  "reference": "s159",
  "schema": "codeclew-documentation-source-part/1.0",
  "snapshot": "sha256:12d7a16bc6dbb7e9c522d3813b06dc095164a5d959c92b46d6cf39958b8e532b/4489",
  "source": {
    "authority": "EXACT_SNAPSHOT_TEXT",
    "endLine": 171,
    "evidenceDigest": "sha256:6d4dc7f870cb655abbff74fc80e09cc2dd99f61975ffdc946cf0252297a00b5a",
    "file": "scripts/build_cli_documentation.py",
    "id": "cli-documentation-dd6a072510e7af3faf22",
    "occurrence": {
      "blob": "a6352ae84b6909b7ab2aacdcf964bf679c1dff78",
      "endByte": 8714,
      "snapshot": "sha256:c315b611c3a5c118f97f5c2d40023779b01e34138cac6334cc086550156a9262",
      "startByte": 7068
    },
    "revision": "d91dbec1164e0601d47c221ab97c504033ce858e",
    "service": "cli-documentation",
    "startLine": 140,
    "textDigest": "sha256:42b270da255c30e3034a074250236d483ed836d285997214e99d40cd078976cf",
    "url": "https://github.com/codeclew/codeclew/blob/d91dbec1164e0601d47c221ab97c504033ce858e/scripts/build_cli_documentation.py#L140-L171"
  },
  "sourceId": "cli-documentation-dd6a072510e7af3faf22",
  "startByte": 0,
  "text": "def main():\n    parser = argparse.ArgumentParser(description=__doc__)\n    parser.add_argument(\"--check\", action=\"store_true\")\n    args = parser.parse_args()\n    data = json.loads(DATA.read_text())\n    claims = verify(data)\n    dot, mermaid = diagram_sources(data)\n    directory = ROOT / \"site/diagrams\"\n    page = ROOT / \"site/nav-query.html\"\n    if args.check:\n        assert (directory / \"nav-query.dot\").read_text() == dot\n        assert (directory / \"nav-query.mmd\").read_text() == mermaid\n        svg = (directory / \"nav-query.svg\").read_text()\n        ET.fromstring(svg)\n        assert svg in page.read_text(), \"page diagram is stale\"\n        assert render_claims(claims) in page.read_text(), \"page claims are stale\"\n        print(f\"PASS: {len(claims)} claims, pinned source digests and rendered graph bindings\")\n        return\n    directory.mkdir(exist_ok=True)\n    svg = render_svg(dot, claims)\n    (directory / \"nav-query.dot\").write_text(dot)\n    (directory / \"nav-query.mmd\").write_text(mermaid)\n    (directory / \"nav-query.svg\").write_text(svg)\n    content = page.read_text()\n    prefix, remainder = content.split(START, 1)\n    _, suffix = remainder.split(END, 1)\n    content = prefix + START + \"\\n\" + svg + \"\\n\" + END + suffix\n    claim_start, claim_end = \"<!-- NAV_QUERY_CLAIMS -->\", \"<!-- /NAV_QUERY_CLAIMS -->\"\n    prefix, remainder = content.split(claim_start, 1)\n    _, suffix = remainder.split(claim_end, 1)\n    page.write_text(prefix + claim_start + \"\\n\" + render_claims(claims) + \"\\n\" + claim_end + suffix)\n    print(f\"Rendered {len(data['diagram']['nodes'])} nodes and {len(data['diagram']['edges'])} evidence-bound arrows\")",
  "totalTextBytes": 1646,
  "work": "530cc649966e8655e58362ea7c779243626fb50977500f92b610738cca8aa724"
}
```

```json
{
  "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
  "endByte": 1539,
  "fragmentDigest": "sha256:5ed1c61da45493ab0dbd06bba764dfd0e35558d3e860638d11ec6b280785e5f0",
  "nextCursor": null,
  "receiptDigest": "sha256:c223cb0837861defed39a47856244c4dc8c0851a1e9c11c8bd2284e41a4168d1",
  "receiptType": "SOURCE_PART",
  "recordDigest": "sha256:14c0e13600d7acb7b7473036ff36ed778d36e476968d481a86acfb64ce0e636b",
  "reference": "s160",
  "schema": "codeclew-documentation-source-part/1.0",
  "snapshot": "sha256:12d7a16bc6dbb7e9c522d3813b06dc095164a5d959c92b46d6cf39958b8e532b/4489",
  "source": {
    "authority": "EXACT_SNAPSHOT_TEXT",
    "endLine": 137,
    "evidenceDigest": "sha256:296f403dfefb6dd9f47cc4ea6af8cc926ce62e143f795f4af7b31c61a4032c65",
    "file": "scripts/build_cli_documentation.py",
    "id": "cli-documentation-de4cd73046421c440a6d",
    "occurrence": {
      "blob": "a6352ae84b6909b7ab2aacdcf964bf679c1dff78",
      "endByte": 7065,
      "snapshot": "sha256:c315b611c3a5c118f97f5c2d40023779b01e34138cac6334cc086550156a9262",
      "startByte": 5526
    },
    "revision": "d91dbec1164e0601d47c221ab97c504033ce858e",
    "service": "cli-documentation",
    "startLine": 109,
    "textDigest": "sha256:5ed1c61da45493ab0dbd06bba764dfd0e35558d3e860638d11ec6b280785e5f0",
    "url": "https://github.com/codeclew/codeclew/blob/d91dbec1164e0601d47c221ab97c504033ce858e/scripts/build_cli_documentation.py#L109-L137"
  },
  "sourceId": "cli-documentation-de4cd73046421c440a6d",
  "startByte": 0,
  "text": "def render_claims(claims):\n    escape = html.escape\n    articles = []\n    for claim in claims.values():\n        sources = []\n        for source in claim[\"evidence\"]:\n            label = f\"{source['file']}:{source['startLine']}–{source['endLine']}\"\n            sources.append(\n                '<div class=\"source-record\">'\n                f'<a href=\"{escape(source[\"url\"])}\">{escape(label)} ↗</a>'\n                '<p class=\"source-authority\">EXACT SNAPSHOT TEXT · RETRIEVED BY CODECLEW</p>'\n                f'<pre><code>{escape(source[\"text\"])}</code></pre>'\n                '<details><summary>Digests and evidence binding</summary>'\n                f'<p>Fragment: {escape(source[\"textDigest\"])}<br>'\n                f'File: {escape(source[\"fileDigest\"])}<br>'\n                f'Context: {escape(source[\"contextId\"])}<br>'\n                f'Evidence: {escape(source[\"evidenceDigest\"])}</p></details></div>'\n            )\n        articles.append(\n            f'<article class=\"claim-panel\" id=\"claim-{claim[\"id\"]}\">'\n            f'<p class=\"claim-id\">claim:{claim[\"id\"]}</p>'\n            f'<h2>{escape(claim[\"title\"])}</h2><p>{escape(claim[\"summary\"])}</p>'\n            f'<p class=\"mechanism\">{escape(claim[\"mechanism\"])}</p>'\n            '<div class=\"claim-boundary\"><b>Evidence boundary</b>'\n            f'<p>{escape(claim[\"boundary\"])}</p></div>'\n            '<details class=\"claim-evidence\"><summary>Inspect supporting code</summary>'\n            + \"\".join(sources) + '</details></article>'\n        )\n    return \"\\n\".join(articles)",
  "totalTextBytes": 1539,
  "work": "530cc649966e8655e58362ea7c779243626fb50977500f92b610738cca8aa724"
}
```

```json
{
  "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
  "endByte": 44,
  "fragmentDigest": "sha256:7c2134afaedc872ad0a7cb591e734cbc130315613b7414516c5a51bc8e1bcc09",
  "nextCursor": null,
  "receiptDigest": "sha256:95c06f463ce4a3db48d57d40d085b1712920f0f342c22f4a466460a271d83a6f",
  "receiptType": "SOURCE_PART",
  "recordDigest": "sha256:46c3fb602ac6c9ba2eb191a06bfa8f7f71337bdda0ea832e9bfe62d926dc9256",
  "reference": "s167",
  "schema": "codeclew-documentation-source-part/1.0",
  "snapshot": "sha256:12d7a16bc6dbb7e9c522d3813b06dc095164a5d959c92b46d6cf39958b8e532b/4489",
  "source": {
    "authority": "EXACT_SNAPSHOT_TEXT",
    "endLine": 19,
    "evidenceDigest": "sha256:ec4ce52745ae36800af232b994c7542721fe09147d53020a46461527e2788330",
    "file": "scripts/build_cli_documentation.py",
    "id": "cli-documentation-ea92c36a8459fe7aa2aa",
    "occurrence": {
      "blob": "a6352ae84b6909b7ab2aacdcf964bf679c1dff78",
      "endByte": 577,
      "snapshot": "sha256:c315b611c3a5c118f97f5c2d40023779b01e34138cac6334cc086550156a9262",
      "startByte": 533
    },
    "revision": "d91dbec1164e0601d47c221ab97c504033ce858e",
    "service": "cli-documentation",
    "startLine": 19,
    "textDigest": "sha256:7c2134afaedc872ad0a7cb591e734cbc130315613b7414516c5a51bc8e1bcc09",
    "url": "https://github.com/codeclew/codeclew/blob/d91dbec1164e0601d47c221ab97c504033ce858e/scripts/build_cli_documentation.py#L19-L19"
  },
  "sourceId": "cli-documentation-ea92c36a8459fe7aa2aa",
  "startByte": 0,
  "text": "DATA = ROOT / \"site/evidence/nav-query.json\"",
  "totalTextBytes": 44,
  "work": "530cc649966e8655e58362ea7c779243626fb50977500f92b610738cca8aa724"
}
```

```json
{
  "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
  "endByte": 17,
  "fragmentDigest": "sha256:7d8752c4c7ea34c2c6d04a15aa31a1f25f62848f8c3c82528fd62439c01a225b",
  "nextCursor": null,
  "receiptDigest": "sha256:300f41bf94103d9dd6716bf6ff5ed6dd8a4d354bf8ec8079bcf627c6c7699643",
  "receiptType": "SOURCE_PART",
  "recordDigest": "sha256:4b24b2730ce1be7fa7aa4f1383aa5c58a69aad4cc01eeb5615ba7557219c8433",
  "reference": "s168",
  "schema": "codeclew-documentation-source-part/1.0",
  "snapshot": "sha256:12d7a16bc6dbb7e9c522d3813b06dc095164a5d959c92b46d6cf39958b8e532b/4489",
  "source": {
    "authority": "EXACT_SNAPSHOT_TEXT",
    "endLine": 15,
    "evidenceDigest": "sha256:63a4dfd99ba53c825465fe1debc3b5cbce835193150ca6980ca6f854250cdde6",
    "file": "scripts/build_cli_documentation.py",
    "id": "cli-documentation-ee52236203471163ac3a",
    "occurrence": {
      "blob": "a6352ae84b6909b7ab2aacdcf964bf679c1dff78",
      "endByte": 453,
      "snapshot": "sha256:c315b611c3a5c118f97f5c2d40023779b01e34138cac6334cc086550156a9262",
      "startByte": 436
    },
    "revision": "d91dbec1164e0601d47c221ab97c504033ce858e",
    "service": "cli-documentation",
    "startLine": 15,
    "textDigest": "sha256:7d8752c4c7ea34c2c6d04a15aa31a1f25f62848f8c3c82528fd62439c01a225b",
    "url": "https://github.com/codeclew/codeclew/blob/d91dbec1164e0601d47c221ab97c504033ce858e/scripts/build_cli_documentation.py#L15-L15"
  },
  "sourceId": "cli-documentation-ee52236203471163ac3a",
  "startByte": 0,
  "text": "import subprocess",
  "totalTextBytes": 17,
  "work": "530cc649966e8655e58362ea7c779243626fb50977500f92b610738cca8aa724"
}
```

```json
{
  "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
  "endByte": 15,
  "fragmentDigest": "sha256:f534a72c58d199602723840a24761409f227f4238759b1f6b1982afb8956b460",
  "nextCursor": null,
  "receiptDigest": "sha256:f5035705b7b769994156e51946666b08c4089a867fe9931e2fc5917675276117",
  "receiptType": "SOURCE_PART",
  "recordDigest": "sha256:066201fa727a0272298bba29f5429113aac9745f45619820a4069117bc58fef2",
  "reference": "s18",
  "schema": "codeclew-documentation-source-part/1.0",
  "snapshot": "sha256:12d7a16bc6dbb7e9c522d3813b06dc095164a5d959c92b46d6cf39958b8e532b/4489",
  "source": {
    "authority": "EXACT_SNAPSHOT_TEXT",
    "endLine": 9,
    "evidenceDigest": "sha256:b6b46c54908589f792573a9b8d1332803d0f741dae1c4c937ea895d9e0c71389",
    "file": "scripts/build_cli_documentation.py",
    "id": "cli-documentation-161984359d1a5ed47c0a",
    "occurrence": {
      "blob": "a6352ae84b6909b7ab2aacdcf964bf679c1dff78",
      "endByte": 361,
      "snapshot": "sha256:c315b611c3a5c118f97f5c2d40023779b01e34138cac6334cc086550156a9262",
      "startByte": 346
    },
    "revision": "d91dbec1164e0601d47c221ab97c504033ce858e",
    "service": "cli-documentation",
    "startLine": 9,
    "textDigest": "sha256:f534a72c58d199602723840a24761409f227f4238759b1f6b1982afb8956b460",
    "url": "https://github.com/codeclew/codeclew/blob/d91dbec1164e0601d47c221ab97c504033ce858e/scripts/build_cli_documentation.py#L9-L9"
  },
  "sourceId": "cli-documentation-161984359d1a5ed47c0a",
  "startByte": 0,
  "text": "import argparse",
  "totalTextBytes": 15,
  "work": "530cc649966e8655e58362ea7c779243626fb50977500f92b610738cca8aa724"
}
```

```json
{
  "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
  "endByte": 42,
  "fragmentDigest": "sha256:368e0b19b5c2054e768091e0f4a14f5009340b92571f3e578aca9185f278b094",
  "nextCursor": null,
  "receiptDigest": "sha256:f7e304d48dba76b8571a6cd4fece43eda369c6ccde573b7809a7c612f21594d6",
  "receiptType": "SOURCE_PART",
  "recordDigest": "sha256:cc2fe691c392e18697e7b64bfa106afdd62a0afcf75c6a0308d402cd8448c9c1",
  "reference": "s19",
  "schema": "codeclew-documentation-source-part/1.0",
  "snapshot": "sha256:12d7a16bc6dbb7e9c522d3813b06dc095164a5d959c92b46d6cf39958b8e532b/4489",
  "source": {
    "authority": "EXACT_SNAPSHOT_TEXT",
    "endLine": 18,
    "evidenceDigest": "sha256:04961c6451f58de7dc84c2ff63796c078ff1bd6bd663bbdc7f53bd713ecb5203",
    "file": "scripts/build_cli_documentation.py",
    "id": "cli-documentation-170d48b2af9e3cd1414c",
    "occurrence": {
      "blob": "a6352ae84b6909b7ab2aacdcf964bf679c1dff78",
      "endByte": 532,
      "snapshot": "sha256:c315b611c3a5c118f97f5c2d40023779b01e34138cac6334cc086550156a9262",
      "startByte": 490
    },
    "revision": "d91dbec1164e0601d47c221ab97c504033ce858e",
    "service": "cli-documentation",
    "startLine": 18,
    "textDigest": "sha256:368e0b19b5c2054e768091e0f4a14f5009340b92571f3e578aca9185f278b094",
    "url": "https://github.com/codeclew/codeclew/blob/d91dbec1164e0601d47c221ab97c504033ce858e/scripts/build_cli_documentation.py#L18-L18"
  },
  "sourceId": "cli-documentation-170d48b2af9e3cd1414c",
  "startByte": 0,
  "text": "ROOT = Path(__file__).resolve().parents[1]",
  "totalTextBytes": 42,
  "work": "530cc649966e8655e58362ea7c779243626fb50977500f92b610738cca8aa724"
}
```

```json
{
  "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
  "endByte": 1952,
  "fragmentDigest": "sha256:5924ef657fba968bdbec239fa07b40f819523fdf00228944209332bcc84d79f3",
  "nextCursor": null,
  "receiptDigest": "sha256:6b64a29400b3f390e895cb0e7ae5e12b7875fbe512b39128d171f411fb68bd60",
  "receiptType": "SOURCE_PART",
  "recordDigest": "sha256:1b709542cd430eecb38117bc024c229472b729a212038dd7387e2ac379002d56",
  "reference": "s34",
  "schema": "codeclew-documentation-source-part/1.0",
  "snapshot": "sha256:12d7a16bc6dbb7e9c522d3813b06dc095164a5d959c92b46d6cf39958b8e532b/4489",
  "source": {
    "authority": "EXACT_SNAPSHOT_TEXT",
    "endLine": 57,
    "evidenceDigest": "sha256:df2fa9378600e189292e6a84bda987013862c3121986f27b79880eacc37ed77c",
    "file": "scripts/build_cli_documentation.py",
    "id": "cli-documentation-30dea2dc5cb47627d15f",
    "occurrence": {
      "blob": "a6352ae84b6909b7ab2aacdcf964bf679c1dff78",
      "endByte": 2601,
      "snapshot": "sha256:c315b611c3a5c118f97f5c2d40023779b01e34138cac6334cc086550156a9262",
      "startByte": 649
    },
    "revision": "d91dbec1164e0601d47c221ab97c504033ce858e",
    "service": "cli-documentation",
    "startLine": 24,
    "textDigest": "sha256:5924ef657fba968bdbec239fa07b40f819523fdf00228944209332bcc84d79f3",
    "url": "https://github.com/codeclew/codeclew/blob/d91dbec1164e0601d47c221ab97c504033ce858e/scripts/build_cli_documentation.py#L24-L57"
  },
  "sourceId": "cli-documentation-30dea2dc5cb47627d15f",
  "startByte": 0,
  "text": "def verify(data):\n    revision = data[\"repositoryRevision\"]\n    assert re.fullmatch(r\"[a-f0-9]{40}\", revision), \"expected an immutable Git revision\"\n    claims = {claim[\"id\"]: claim for claim in data[\"claims\"]}\n    assert len(claims) == len(data[\"claims\"]), \"duplicate claim IDs\"\n    blobs = {}\n    for claim in claims.values():\n        assert claim[\"narrativeAuthority\"] == \"AGENT_INFERRED\"\n        assert claim[\"evidence\"], f\"missing evidence: {claim['id']}\"\n        for source in claim[\"evidence\"]:\n            path = source[\"file\"]\n            assert not Path(path).is_absolute() and \"..\" not in Path(path).parts\n            if path not in blobs:\n                blobs[path] = subprocess.check_output(\n                    [\"git\", \"show\", f\"{revision}:{path}\"], cwd=ROOT\n                )\n            blob = blobs[path]\n            assert \"sha256:\" + hashlib.sha256(b\"codeclew-cas/v2\\0\" + b\"codeclew-repository-input-blob/2.0\\0\" + blob).hexdigest() == source[\"fileDigest\"]\n            lines = blob.decode(\"utf-8\").splitlines()\n            start, end = source[\"startLine\"], source[\"endLine\"]\n            assert 1 <= start <= end <= len(lines)\n            text = \"\\n\".join(lines[start - 1:end])\n            assert text == source[\"text\"], f\"source mismatch: {claim['id']} {path}:{start}\"\n            assert \"sha256:\" + hashlib.sha256(text.encode()).hexdigest() == source[\"textDigest\"]\n            assert source[\"authority\"] == \"EXACT_SNAPSHOT_TEXT\"\n            assert source[\"url\"] == f\"{data['repository']}/blob/{revision}/{path}#L{start}-L{end}\"\n            assert re.fullmatch(r\"sha256:[a-f0-9]{64}\", source[\"evidenceDigest\"])\n    nodes = {node[\"id\"] for node in data[\"diagram\"][\"nodes\"]}\n    assert nodes <= claims.keys()\n    for edge in data[\"diagram\"][\"edges\"]:\n        assert edge[\"from\"] in nodes and edge[\"to\"] in nodes\n        assert edge[\"claimId\"] in claims\n        assert edge[\"authority\"] == \"AGENT_INFERRED_STATIC_FLOW\"\n    return claims",
  "totalTextBytes": 1952,
  "work": "530cc649966e8655e58362ea7c779243626fb50977500f92b610738cca8aa724"
}
```

```json
{
  "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED",
  "endByte": 11,
  "fragmentDigest": "sha256:ff3e4d4dcf7250cae8d622d77c9a6d69aefcf4ab04ef9920218fafe93c0f1991",
  "nextCursor": null,
  "receiptDigest": "sha256:0c1ff59ac3e5f4e95e147934d802a275e517d3914ab07203a8740ea37632d5c2",
  "receiptType": "SOURCE_PART",
  "recordDigest": "sha256:526634853263e023c0dde5a0ece703864de92ab828f967fff56144548f623e1d",
  "reference": "s77",
  "schema": "codeclew-documentation-source-part/1.0",
  "snapshot": "sha256:12d7a16bc6dbb7e9c522d3813b06dc095164a5d959c92b46d6cf39958b8e532b/4489",
  "source": {
    "authority": "EXACT_SNAPSHOT_TEXT",
    "endLine": 12,
    "evidenceDigest": "sha256:3672dc39080014382a21db12831f6c65e971d1cf79e826db099b8ae626491651",
    "file": "scripts/build_cli_documentation.py",
    "id": "cli-documentation-71b0b2a7e67d7924bbdd",
    "occurrence": {
      "blob": "a6352ae84b6909b7ab2aacdcf964bf679c1dff78",
      "endByte": 400,
      "snapshot": "sha256:c315b611c3a5c118f97f5c2d40023779b01e34138cac6334cc086550156a9262",
      "startByte": 389
    },
    "revision": "d91dbec1164e0601d47c221ab97c504033ce858e",
    "service": "cli-documentation",
    "startLine": 12,
    "textDigest": "sha256:ff3e4d4dcf7250cae8d622d77c9a6d69aefcf4ab04ef9920218fafe93c0f1991",
    "url": "https://github.com/codeclew/codeclew/blob/d91dbec1164e0601d47c221ab97c504033ce858e/scripts/build_cli_documentation.py#L12-L12"
  },
  "sourceId": "cli-documentation-71b0b2a7e67d7924bbdd",
  "startByte": 0,
  "text": "import json",
  "totalTextBytes": 11,
  "work": "530cc649966e8655e58362ea7c779243626fb50977500f92b610738cca8aa724"
}
```
