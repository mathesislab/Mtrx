package org.mtrx.app.timeline

import android.graphics.Paint
import android.graphics.Typeface
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.InlineTextContent
import androidx.compose.foundation.text.appendInlineContent
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.Placeholder
import androidx.compose.ui.text.PlaceholderVerticalAlign
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.BaselineShift
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import kotlin.math.max
import kotlin.math.roundToInt
import org.mtrx.app.model.Images
import org.mtrx.app.model.TypesetFormula
import uniffi.mtrx.BlockKind
import uniffi.mtrx.Formula
import uniffi.mtrx.TextBlock
import uniffi.mtrx.TextRun
import uniffi.mtrx.TextStyle as RunStyle

/**
 * What runs look like besides their style: the [text] colour (code
 * backgrounds and hidden spoilers are shades of it), [link]s and mentions,
 * the [muted] "(edited)", and whether the spoilers are shown.
 */
internal data class TextLook(val text: Color, val link: Color, val muted: Color, val spoilersShown: Boolean = false)

/** What touching a message's text does. */
internal class TextLinks(
    val openLink: (String) -> Unit,
    val showPerson: (userId: String, name: String) -> Unit,
    val showSpoilers: () -> Unit,
)

/**
 * A message's text ([uniffi.mtrx.Message.text]), as the library finished
 * it: blocks one above the other with their quote bars, list indents and
 * markers. A paragraph is one text of styled runs with its formulas
 * typeset inline (`$tex$` in monospace until they are, or if they cannot
 * be); display math is centred, scaled down to 60 % to fit, then scrolls.
 * Links open with [onOpenLink], mentions show the person, a spoiler
 * shows the message's spoilers; holding is the bubble's.
 */
@Composable
internal fun MessageText(
    blocks: List<TextBlock>,
    edited: Boolean,
    look: TextLook,
    images: Images,
    onOpenLink: (String) -> Unit,
    onShowPerson: (userId: String, name: String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val body = MaterialTheme.typography.bodyLarge
    val emPx = with(LocalDensity.current) { body.fontSize.toPx() }.roundToInt()
    val formulas = remember(blocks) { formulas(blocks) }
    // Composed rows only: those on screen.
    val typeset by produceState(remember(formulas, emPx) { cached(formulas, emPx, images) }, formulas, emPx) {
        for (formula in formulas) {
            if (formula in value) continue
            images.formula(formula, emPx)?.let { value = value + (formula to it) }
        }
    }
    var spoilersShown by remember(blocks) { mutableStateOf(false) }
    val shownLook = look.copy(spoilersShown = spoilersShown)
    val links = remember(onOpenLink, onShowPerson) {
        TextLinks(onOpenLink, onShowPerson, showSpoilers = { spoilersShown = true })
    }
    Column(modifier, verticalArrangement = Arrangement.spacedBy(6.dp)) {
        blocks.forEachIndexed { index, block ->
            TextBlockView(block, edited && index == blocks.lastIndex, shownLook, links, typeset)
        }
        if (edited && !editedInline(blocks)) {
            Text(EDITED.trim(), style = MaterialTheme.typography.bodySmall, color = look.muted)
        }
    }
}

private fun cached(formulas: List<Formula>, emPx: Int, images: Images): Map<Formula, TypesetFormula> =
    formulas.mapNotNull { formula -> images.cachedFormula(formula, emPx)?.let { formula to it } }.toMap()

/** A block, indented for its lists, quote bars at its left; [edited] ends it with "(edited)". */
@Composable
private fun TextBlockView(
    block: TextBlock,
    edited: Boolean,
    look: TextLook,
    links: TextLinks,
    typeset: Map<Formula, TypesetFormula>,
) {
    val quotes = block.quotes.toInt()
    val lists = block.lists.toInt()
    Row(
        Modifier
            .drawBehind {
                for (bar in 0 until quotes) {
                    drawRect(look.muted, Offset(bar * QUOTE_STEP.toPx(), 0f), Size(QUOTE_BAR.toPx(), size.height))
                }
            }
            .padding(start = QUOTE_STEP * quotes + LIST_STEP * max(lists - 1, 0)),
    ) {
        if (lists > 0) {
            Text(block.marker.orEmpty(), Modifier.widthIn(min = 20.dp), color = look.text)
        }
        when (val kind = block.kind) {
            is BlockKind.Paragraph -> Paragraph(kind.runs, edited, look, links, typeset)
            is BlockKind.Heading -> Paragraph(kind.runs, edited, look, links, typeset, heading = true)
            is BlockKind.Code -> Text(
                kind.code,
                Modifier
                    .background(look.text.copy(alpha = 0.15f), RoundedCornerShape(8.dp))
                    .padding(horizontal = 8.dp, vertical = 6.dp),
                color = look.text,
                fontFamily = FontFamily.Monospace,
                style = MaterialTheme.typography.bodyMedium,
            )
            is BlockKind.Math -> when (val image = typeset[kind.formula]) {
                null -> Text(fallback(kind.formula), color = look.text, fontFamily = FontFamily.Monospace)
                else -> DisplayFormula(image, kind.formula.tex, look.text)
            }
            BlockKind.Rule -> HorizontalDivider(Modifier.padding(vertical = 4.dp), color = look.muted)
        }
    }
}

/** A paragraph: runs, inline formulas placed on the baseline. */
@Composable
private fun Paragraph(
    runs: List<TextRun>,
    edited: Boolean,
    look: TextLook,
    links: TextLinks,
    typeset: Map<Formula, TypesetFormula>,
    heading: Boolean = false,
) {
    val style = if (heading) {
        MaterialTheme.typography.titleMedium.copy(fontWeight = FontWeight.Bold)
    } else {
        MaterialTheme.typography.bodyLarge
    }
    val density = LocalDensity.current
    val text = remember(runs, edited, look, links, typeset.keys) { annotated(runs, edited, look, links) { it in typeset } }
    val inline = remember(runs, typeset, look.text, style.fontSize, density) {
        val fontPx = with(density) { style.fontSize.toPx() }
        val center = textCenter(fontPx)
        runs.mapNotNull { run -> (run as? TextRun.Math)?.formula }
            .mapNotNull { formula -> typeset[formula]?.let { formulaId(formula) to inlineFormula(it, center, density, look.text) } }
            .toMap()
    }
    Text(text, inlineContent = inline, color = look.text, style = style)
}

/**
 * A placeholder for [image] whose baseline lands on the text's: centred
 * on the text's centre ([center] pixels below the baseline, negative above),
 * tall enough to hold the image above and below it.
 */
private fun inlineFormula(image: TypesetFormula, center: Float, density: Density, tint: Color): InlineTextContent {
    val height = image.bitmap.height.toFloat()
    val baseline = image.baseline.toFloat()
    val half = max(baseline + center, height - baseline - center)
    val top = (half - baseline - center).roundToInt()
    return with(density) {
        InlineTextContent(
            Placeholder(image.bitmap.width.toFloat().toSp(), (2 * half).toSp(), PlaceholderVerticalAlign.TextCenter),
        ) {
            val bitmap = remember(image) { image.bitmap.asImageBitmap() }
            Box(Modifier.fillMaxSize()) {
                Image(
                    bitmap,
                    contentDescription = null,
                    colorFilter = ColorFilter.tint(tint),
                    modifier = Modifier
                        .offset { IntOffset(0, top) }
                        .size(image.bitmap.width.toDp(), height.toDp()),
                )
            }
        }
    }
}

/** Where the default font's text centre is, in pixels below the baseline (negative: above), at [fontPx]. */
private fun textCenter(fontPx: Float): Float {
    val metrics = Paint().apply {
        textSize = fontPx
        typeface = Typeface.DEFAULT
    }.fontMetrics ?: return 0f
    return (metrics.ascent + metrics.descent) / 2
}

/** Display math, centred: scaled down to fit, to 60 % at most, then scrolling sideways. */
@Composable
private fun DisplayFormula(image: TypesetFormula, tex: String, tint: Color) {
    val bitmap = remember(image) { image.bitmap.asImageBitmap() }
    val density = LocalDensity.current
    val width = with(density) { image.bitmap.width.toDp() }
    val height = with(density) { image.bitmap.height.toDp() }
    BoxWithConstraints(
        Modifier
            .fillMaxWidth()
            .semantics { contentDescription = tex },
        contentAlignment = Alignment.Center,
    ) {
        val scale = (maxWidth / width).coerceAtMost(1f)
        val shown = Modifier.size(width * max(scale, 0.6f), height * max(scale, 0.6f))
        val picture = @Composable { modifier: Modifier ->
            Image(bitmap, contentDescription = null, modifier = modifier, colorFilter = ColorFilter.tint(tint))
        }
        if (scale >= 0.6f) {
            picture(shown)
        } else {
            Box(Modifier.horizontalScroll(rememberScrollState())) { picture(shown) }
        }
    }
}

private val QUOTE_BAR = 3.dp
private val QUOTE_STEP = 11.dp
private val LIST_STEP = 16.dp
private const val EDITED = " (edited)"

/** The id of a formula's inline content. */
internal fun formulaId(formula: Formula) = "formula:${formula.tex}"

/** A formula as text, before it is typeset or if it cannot be. */
internal fun fallback(formula: Formula) = if (formula.display) "$$${formula.tex}$$" else "$${formula.tex}$"

/** Every formula of [blocks], inline or displayed, in order. */
internal fun formulas(blocks: List<TextBlock>): List<Formula> = blocks.flatMap { block ->
    when (val kind = block.kind) {
        is BlockKind.Paragraph -> kind.runs.mapNotNull { (it as? TextRun.Math)?.formula }
        is BlockKind.Heading -> kind.runs.mapNotNull { (it as? TextRun.Math)?.formula }
        is BlockKind.Math -> listOf(kind.formula)
        else -> emptyList()
    }
}

/** "(edited)" goes after the last paragraph's text, else on a line of its own (after code, math or a rule). */
internal fun editedInline(blocks: List<TextBlock>): Boolean =
    blocks.lastOrNull()?.kind.let { it is BlockKind.Paragraph || it is BlockKind.Heading }

/** The link a run is in. */
private val TextRun.link: String?
    get() = when (this) {
        is TextRun.Text -> link
        is TextRun.Code -> link
        is TextRun.Math -> link
        is TextRun.Mention -> null
    }

/**
 * [runs] as text: styles as spans, links (consecutive runs in one link
 * together) and mentions as link annotations, formulas [typeset] has as
 * inline content ([formulaId]), others as `$tex$` code; "(edited)" after
 * them if [edited].
 */
internal fun annotated(
    runs: List<TextRun>,
    edited: Boolean,
    look: TextLook,
    links: TextLinks,
    typeset: (Formula) -> Boolean,
): AnnotatedString = buildAnnotatedString {
    var i = 0
    while (i < runs.size) {
        val url = runs[i].link
        if (url == null) {
            appendRun(runs[i], look, links, typeset, linked = false)
            i++
            continue
        }
        var end = i
        while (end < runs.size && runs[end].link == url) end++
        val linkStyle = TextLinkStyles(SpanStyle(color = look.link, textDecoration = TextDecoration.Underline))
        withLink(LinkAnnotation.Url(url, linkStyle) { links.openLink(url) }) {
            for (k in i until end) appendRun(runs[k], look, links, typeset, linked = true)
        }
        i = end
    }
    if (edited) {
        withStyle(SpanStyle(color = look.muted, fontSize = 0.8.em, fontStyle = FontStyle.Normal)) { append(EDITED) }
    }
}

/** [run]; [linked] if it is in a link (a spoiler in it is not tapped apart). */
private fun AnnotatedString.Builder.appendRun(
    run: TextRun,
    look: TextLook,
    links: TextLinks,
    typeset: (Formula) -> Boolean,
    linked: Boolean,
) {
    when (run) {
        is TextRun.Text -> styled(run.style, look, links, linked) { append(run.text) }
        is TextRun.Code -> styled(run.style, look, links, linked) { code(look) { append(run.code) } }
        is TextRun.Math -> when {
            typeset(run.formula) -> appendInlineContent(formulaId(run.formula), run.formula.tex)
            else -> code(look) { append(fallback(run.formula)) }
        }
        is TextRun.Mention -> {
            val bold = SpanStyle(color = look.link, fontWeight = if (run.you) FontWeight.Bold else null)
            withLink(LinkAnnotation.Clickable(PERSON + run.userId, TextLinkStyles(bold)) { links.showPerson(run.userId, run.name) }) {
                styled(run.style, look, links, linked = true) { append(run.name) }
            }
        }
    }
}

private fun AnnotatedString.Builder.code(look: TextLook, content: AnnotatedString.Builder.() -> Unit) {
    withStyle(SpanStyle(fontFamily = FontFamily.Monospace, background = look.text.copy(alpha = 0.15f)), content)
}

/**
 * [content] in [style]; a hidden spoiler is a filled box, tapped to show
 * the spoilers (unless [linked]: links do not nest).
 */
private fun AnnotatedString.Builder.styled(
    style: RunStyle,
    look: TextLook,
    links: TextLinks,
    linked: Boolean,
    content: AnnotatedString.Builder.() -> Unit,
) {
    val decorations = listOfNotNull(
        TextDecoration.LineThrough.takeIf { style.strikethrough },
        TextDecoration.Underline.takeIf { style.underline },
    )
    val span = SpanStyle(
        fontWeight = if (style.bold) FontWeight.Bold else null,
        fontStyle = if (style.italic) FontStyle.Italic else null,
        textDecoration = if (decorations.isEmpty()) null else TextDecoration.combine(decorations),
        baselineShift = when {
            style.superscript -> BaselineShift.Superscript
            style.subscript -> BaselineShift.Subscript
            else -> null
        },
        fontSize = if (style.superscript || style.subscript) 0.75.em else TextUnit.Unspecified,
    )
    withStyle(span) {
        val box = look.text.copy(alpha = 0.35f)
        when {
            !style.spoiler || look.spoilersShown -> content()
            linked -> withStyle(SpanStyle(color = box, background = box), content)
            else -> withLink(
                LinkAnnotation.Clickable(SPOILER, TextLinkStyles(SpanStyle(color = box, background = box))) { links.showSpoilers() },
                content,
            )
        }
    }
}

/** Link tags: a mention's (with the user ID after it), a hidden spoiler's. */
internal const val PERSON = "person:"
internal const val SPOILER = "spoiler"
