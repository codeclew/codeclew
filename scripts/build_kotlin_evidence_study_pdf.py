#!/usr/bin/env python3
from pathlib import Path

from reportlab.lib import colors
from reportlab.lib.enums import TA_CENTER, TA_LEFT
from reportlab.lib.pagesizes import A4
from reportlab.lib.styles import ParagraphStyle, getSampleStyleSheet
from reportlab.lib.units import mm
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.platypus import (
    BaseDocTemplate,
    Frame,
    KeepTogether,
    PageBreak,
    PageTemplate,
    Paragraph,
    Preformatted,
    Spacer,
    Table,
    TableStyle,
)

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "output" / "pdf" / "codeclew-kotlin-evidence-study.pdf"

GREEN = colors.HexColor("#7CCB35")
INK = colors.HexColor("#172018")
MUTED = colors.HexColor("#5D685F")
PALE = colors.HexColor("#F2F7EE")
LINE = colors.HexColor("#D6DFD2")
AMBER = colors.HexColor("#B86B16")


def register_fonts():
    candidates = [
        ("/System/Library/Fonts/Supplemental/Arial.ttf", "Arial"),
        ("/System/Library/Fonts/Supplemental/Arial Bold.ttf", "Arial-Bold"),
        ("/System/Library/Fonts/Supplemental/Courier New.ttf", "Code"),
    ]
    for path, name in candidates:
        if Path(path).exists():
            pdfmetrics.registerFont(TTFont(name, path))
    return (
        "Arial" if "Arial" in pdfmetrics.getRegisteredFontNames() else "Helvetica",
        "Arial-Bold" if "Arial-Bold" in pdfmetrics.getRegisteredFontNames() else "Helvetica-Bold",
        "Code" if "Code" in pdfmetrics.getRegisteredFontNames() else "Courier",
    )


BODY_FONT, BOLD_FONT, CODE_FONT = register_fonts()


def footer(canvas, doc):
    canvas.saveState()
    canvas.setStrokeColor(LINE)
    canvas.line(18 * mm, 14 * mm, 192 * mm, 14 * mm)
    canvas.setFont(BODY_FONT, 8)
    canvas.setFillColor(MUTED)
    canvas.drawString(18 * mm, 9 * mm, "Codeclew Kotlin evidence study · 1 September 2026")
    canvas.drawRightString(192 * mm, 9 * mm, f"{doc.page}")
    canvas.restoreState()


doc = BaseDocTemplate(
    str(OUTPUT),
    pagesize=A4,
    rightMargin=18 * mm,
    leftMargin=18 * mm,
    topMargin=19 * mm,
    bottomMargin=20 * mm,
    title="Evidence Before Guesswork: A Small Kotlin Engineering Study",
    author="Codeclew project",
)
frame = Frame(doc.leftMargin, doc.bottomMargin, doc.width, doc.height, id="normal")
doc.addPageTemplates(PageTemplate(id="study", frames=frame, onPage=footer))

styles = getSampleStyleSheet()
title = ParagraphStyle("Title", parent=styles["Title"], fontName=BOLD_FONT, fontSize=27, leading=30, textColor=INK, alignment=TA_LEFT, spaceAfter=8)
subtitle = ParagraphStyle("Subtitle", parent=styles["BodyText"], fontName=BODY_FONT, fontSize=12, leading=17, textColor=MUTED, spaceAfter=15)
h1 = ParagraphStyle("H1", parent=styles["Heading1"], fontName=BOLD_FONT, fontSize=19, leading=23, textColor=INK, spaceBefore=8, spaceAfter=10)
h2 = ParagraphStyle("H2", parent=styles["Heading2"], fontName=BOLD_FONT, fontSize=12, leading=15, textColor=GREEN, spaceBefore=8, spaceAfter=5)
body = ParagraphStyle("Body", parent=styles["BodyText"], fontName=BODY_FONT, fontSize=9.5, leading=14, textColor=INK, spaceAfter=7)
small = ParagraphStyle("Small", parent=body, fontSize=8, leading=11, textColor=MUTED)
table_header = ParagraphStyle("TableHeader", parent=small, fontName=BOLD_FONT, textColor=colors.white)
callout = ParagraphStyle("Callout", parent=body, fontName=BOLD_FONT, fontSize=13, leading=18, textColor=INK, borderColor=GREEN, borderWidth=1.5, borderPadding=12, backColor=PALE, spaceAfter=15)
code = ParagraphStyle("Code", fontName=CODE_FONT, fontSize=7.4, leading=10, textColor=INK, leftIndent=8, rightIndent=8, borderColor=LINE, borderWidth=.7, borderPadding=8, backColor=colors.HexColor("#F8FAF7"), spaceAfter=9)


def P(text, style=body):
    return Paragraph(text, style)


def metric(value, label, note):
    return [P(value, ParagraphStyle("Metric", parent=title, fontSize=25, leading=27, textColor=GREEN, alignment=TA_CENTER)), P(label, ParagraphStyle("MetricLabel", parent=body, fontName=BOLD_FONT, alignment=TA_CENTER)), P(note, ParagraphStyle("MetricNote", parent=small, alignment=TA_CENTER))]


story = []
story += [P("EVIDENCE BEFORE GUESSWORK", ParagraphStyle("Kicker", parent=small, fontName=BOLD_FONT, textColor=GREEN, spaceAfter=5)), P("A small Kotlin engineering study", title), P("What Codeclew can already prove for an agent, where plain text search remains sufficient, and what is still not established.", subtitle)]

metric_table = Table([
    [metric("5/5", "engineering concerns covered", "compatibility, coordinates, Maven, CFG, manifest"), metric("16/16", "focused Kotlin checks passed", "zero failures in the frozen slice"), metric("1", "bounded RELEASE receipt", "exact source + K2 declaration identity")]
], colWidths=[doc.width / 3] * 3)
metric_table.setStyle(TableStyle([
    ("VALIGN", (0, 0), (-1, -1), "TOP"),
    ("BOX", (0, 0), (-1, -1), .8, LINE),
    ("INNERGRID", (0, 0), (-1, -1), .8, LINE),
    ("BACKGROUND", (0, 0), (-1, -1), PALE),
    ("LEFTPADDING", (0, 0), (-1, -1), 10),
    ("RIGHTPADDING", (0, 0), (-1, -1), 10),
    ("TOPPADDING", (0, 0), (-1, -1), 13),
    ("BOTTOMPADDING", (0, 0), (-1, -1), 13),
]))
story += [metric_table, Spacer(1, 13), P("Sample-scoped claim", h2), Spacer(1, 5), P("On five representative concerns in Codeclew's public Kotlin 2.4 worker, every selected mechanism had an exact source fragment and a focused executable check. Sixteen selected tests passed. A preserved warm RELEASE navigation receipt additionally returned the complete decision source, a compiler-resolved declaration identity, exact revision authority and content digests in one bounded response.", callout)]
story += [P("This supports a practical claim: <b>Codeclew can give an agent decision-ready, freshness-bound Kotlin evidence instead of only a list of textual matches.</b> It does not establish a population-wide success-rate or token advantage.", body)]

story += [PageBreak(), P("1 · Study design", h1), P("The study was intentionally small. It avoids another expensive multi-agent benchmark and asks whether the released Kotlin contour exposes useful evidence across several common engineering concerns.", body)]
rows = [[P("Slice", table_header), P("Engineering question", table_header), P("Executable evidence", table_header)]]
data = [
    ("K1", "How is a project/compiler pair admitted or rejected?", "4 compatibility-policy tests"),
    ("K2", "Are FIR UTF-16 offsets safe after Cyrillic and emoji?", "6 coordinate-normalization tests"),
    ("K3", "Can Maven module selection escape or become ambiguous?", "1 reactor-selection test"),
    ("K4", "Does an oversized or malformed control-flow graph fail closed?", "3 local-CFG tests"),
    ("K5", "Is semantic input authority canonical and replaceable?", "2 manifest-authority tests"),
]
for row in data:
    rows.append([P(row[0], body), P(row[1], body), P(row[2], body)])
table = Table(rows, colWidths=[16 * mm, 101 * mm, 57 * mm], repeatRows=1)
table.setStyle(TableStyle([
    ("BACKGROUND", (0, 0), (-1, 0), INK), ("TEXTCOLOR", (0, 0), (-1, 0), colors.white),
    ("GRID", (0, 0), (-1, -1), .6, LINE), ("VALIGN", (0, 0), (-1, -1), "TOP"),
    ("LEFTPADDING", (0, 0), (-1, -1), 7), ("RIGHTPADDING", (0, 0), (-1, -1), 7),
    ("TOPPADDING", (0, 0), (-1, -1), 7), ("BOTTOMPADDING", (0, 0), (-1, -1), 7),
]))
story += [table, Spacer(1, 10), P("Selection and execution", h2), P("The five concerns were selected manually for engineering relevance before the focused green run. They are not random or held out. The source authority is public commit <font name='Code'>6281138</font>. The focused Gradle invocation ran warm after compilation and reported 16 tests, 0 skipped, 0 failed. Test reports and source digests were checked locally before this article was built.", body)]
story += [P("Text-search baseline", h2), P("For the same terms, a deterministic <font name='Code'>rg -n</font> baseline returned 160 matching lines across 11 file-task combinations: K1 5 lines/1 file, K2 21/2, K3 82/2, K4 22/2, and K5 30/4. This is not a speed contest: <font name='Code'>rg</font> is excellent for exact strings. The comparison asks what the result means. Text search supplies lexical locations; Codeclew can additionally bind a selected declaration, compilation, exact source window and evidence digest.", body)]

story += [PageBreak(), P("2 · What the Kotlin evidence looks like", h1)]
examples = [
    ("Compatibility decisions are typed", 'val row = QUALIFIED_KOTLIN_ENGINE_ROWS.singleOrNull {\n    it.projectCompilerVersion == project.projectCompilerVersion &&\n        it.engineCompilerVersion == engine.analyzerCompilerVersion\n} ?: return KotlinEngineCompatibilityDecision(\n    status = "REJECTED",\n    kind = "UNQUALIFIED",\n    reason = "PROJECT_ENGINE_ROW_NOT_QUALIFIED",\n    btaEligible = false,\n)'),
    ("Nested positions share one byte domain", 'val byteRange = coordinates.range(start, end) ?: return null\nnormalized += rebuild(argument, mapOf(\n    "argumentStart" to JsonPrimitive(byteRange.first),\n    "argumentEnd" to JsonPrimitive(byteRange.last + 1),\n))'),
    ("Oversized CFGs become boundaries", 'if (rawNodes.size > 4_096 || rawEdges.size > 8_192) {\n    return localCfgBoundary(\n        "LOCAL_CFG_BUDGET_EXCEEDED", ...\n    )\n}'),
]
for heading, snippet in examples:
    story += [P(heading, h2), Preformatted(snippet, code)]
story += [P("These excerpts are not free-floating screenshots. The public evidence record links each fragment to a repository revision, file, line range and SHA-256 source digest. On the interactive documentation page, selecting a fact changes the exact code block shown beside it.", body)]

story += [PageBreak(), P("3 · Results and interpretation", h1)]
result_rows = [[P("Observation", table_header), P("Result", table_header), P("Interpretation", table_header)]] + [
    [P("Focused correctness", body), P("16/16 passed", body), P("All selected mechanisms have executable support at the frozen revision.", body)],
    [P("Concern coverage", body), P("5/5", body), P("The sample spans policy, coordinates, build topology, bounded graphs and hashing.", body)],
    [P("Warm RELEASE navigation", body), P("1 bounded response", body), P("Returned 3 ranked cards, one exact 115-line source window, K2 identity and evidence digest.", body)],
    [P("Plain text baseline", body), P("160 matching lines", body), P("Useful lexical discovery, but no compiler identity or freshness authority.", body)],
    [P("Prior paired agent task", body), P("3.795× fewer effective tokens", body), P("Both answers were source-correct; this remains one prepared warm task.", body)],
]
results = Table(result_rows, colWidths=[44 * mm, 46 * mm, 84 * mm], repeatRows=1)
results.setStyle(TableStyle([
    ("BACKGROUND", (0, 0), (-1, 0), INK), ("TEXTCOLOR", (0, 0), (-1, 0), colors.white),
    ("GRID", (0, 0), (-1, -1), .6, LINE), ("VALIGN", (0, 0), (-1, -1), "TOP"),
    ("LEFTPADDING", (0, 0), (-1, -1), 7), ("RIGHTPADDING", (0, 0), (-1, -1), 7),
    ("TOPPADDING", (0, 0), (-1, -1), 7), ("BOTTOMPADDING", (0, 0), (-1, -1), 7),
]))
story += [results, Spacer(1, 10), P("Where Codeclew adds value", h2), P("The advantage is strongest when an agent must choose among similarly named declarations, explain a branch with its exact qualifiers, or carry evidence into a later change. A bounded card can contain the selected declaration, a source window, compilation authority, certainty, limits and a stable digest. That is more decision-ready than a long list of textual hits and safer than an uncited summary.", body)]
story += [P("Where <font name='Code'>rg</font> remains the right tool", h2), P("For an exact string in a known file, <font name='Code'>rg</font> is simpler and faster. Codeclew should not replace it. The product claim is narrower: Codeclew is useful when navigation requires identity, context, boundedness and evidence that survives review.", body)]

story += [PageBreak(), P("4 · Limitations and next evidence", h1)]
limits = [
    ("One public module", "The five concerns come from Codeclew's Kotlin worker, not five independent repositories."),
    ("Manual selection", "The sample demonstrates breadth but cannot estimate population performance."),
    ("Warm focused run", "Cold compilation and installation were excluded; the 784 ms result is not a cold-start claim."),
    ("One navigation receipt", "Only the preserved compatibility-policy query is used as direct RELEASE navigation evidence."),
    ("Fresh-run operational blocker", "A new attempt on 1 September stopped before facts with RESOURCE_LIMIT because the long-lived local CAS catalog snapshot exceeded its bound. The failed attempt is not counted as a successful case."),
    ("Prior token result remains descriptive", "The 3.795× token result and 40.28× tool-output result come from one prepared warm paired task and do not prove general superiority."),
]
for name, explanation in limits:
    story.append(KeepTogether([P(name, h2), P(explanation, body)]))
story += [Spacer(1, 6), P("Minimal next gate", h2), Spacer(1, 5), P("For a stronger public claim, freeze six small tasks across three real Kotlin repositories: two location/explanation tasks per repository, balanced Default/Codeclew order, identical model and prompt budget, warm state on both arms, and a blinded source-correctness check. Stop after six tasks. That is enough to test whether the observed advantage transfers without rebuilding an industrial benchmark harness.", callout)]

story += [PageBreak(), P("5 · Evidence ledger", h1)]
ledger = [
    ("Source revision", "6281138ecbf73bc5de1a9c7eaeb2cdf7009e6ca1"),
    ("Kotlin profile", "kotlin-2.4.10-gradle-single"),
    ("Compilation", ":workers:kotlin/main"),
    ("Focused test result", "16 tests / 0 failures / 0 skipped"),
    ("Worker.kt SHA-256", "b4a5bdd058e9d400e2955dbc8115893b0911909fc446abc5a08c5c86992d4e44"),
    ("MavenProjectModel.kt SHA-256", "9c5ae9638c36e7072d50cce1f2518f5672e42a4075dda2d543404310cf6b386e"),
    ("LocalCfgIndex.kt SHA-256", "07dd66f68183c25df4d56b3b814769bdd8f898a490d3e368e2e8f7a1fd8a06e3"),
    ("SemanticInputManifestAuthority.kt SHA-256", "8dbffc93e43a5ba236f91229f99c7343ff64cfdac4c270f1c5150c98529d7ea4"),
    ("Preserved navigation receipt SHA-256", "ea21155aa47f533394a64e67b6f8dda29e1b0f98d27ddf29e6af24414e4b85b4"),
    ("Paired Q1 evidence receipt SHA-256", "61ec5bcc2583b09e9d1c6d4539055e46031762118b62bbe87de4aa68da808ab2"),
]
ledger_table = Table([[P(k, small), P(v, ParagraphStyle("Ledger", parent=small, fontName=CODE_FONT, fontSize=7))] for k, v in ledger], colWidths=[57 * mm, 117 * mm])
ledger_table.setStyle(TableStyle([
    ("GRID", (0, 0), (-1, -1), .6, LINE), ("VALIGN", (0, 0), (-1, -1), "TOP"),
    ("BACKGROUND", (0, 0), (0, -1), PALE),
    ("LEFTPADDING", (0, 0), (-1, -1), 7), ("RIGHTPADDING", (0, 0), (-1, -1), 7),
    ("TOPPADDING", (0, 0), (-1, -1), 6), ("BOTTOMPADDING", (0, 0), (-1, -1), 6),
]))
story += [ledger_table, Spacer(1, 12), P("Conclusion", h2), P("The small study justifies using Codeclew today for bounded Kotlin evidence navigation in prepared projects. It does not justify removing text search, claiming universal token savings, or ignoring the catalog resource-limit blocker. The useful product direction is clear: keep the response compact and evidence-bound, make accumulated-state maintenance reliable, and validate transfer on a six-task, three-repository follow-up.", body)]

OUTPUT.parent.mkdir(parents=True, exist_ok=True)
doc.build(story)
print(OUTPUT)
