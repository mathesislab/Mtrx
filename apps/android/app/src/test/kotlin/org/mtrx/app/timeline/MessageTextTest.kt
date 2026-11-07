package org.mtrx.app.timeline

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.mtrx.app.PLAIN
import org.mtrx.app.paragraph
import org.mtrx.app.plain
import uniffi.mtrx.BlockKind
import uniffi.mtrx.Formula
import uniffi.mtrx.TextBlock
import uniffi.mtrx.TextRun

class MessageTextTest {
    private val look = TextLook(text = Color.Black, link = Color.Blue, muted = Color.Gray)
    private val opened = mutableListOf<String>()
    private val people = mutableListOf<Pair<String, String>>()
    private var spoilers = 0
    private val links = TextLinks({ opened += it }, { id, name -> people += id to name }, { spoilers++ })

    private fun text(vararg runs: TextRun, edited: Boolean = false, typeset: (Formula) -> Boolean = { false }) =
        annotated(runs.toList(), edited, look, links, typeset)

    /** The link annotations, each with the text it covers. */
    private fun AnnotatedString.linked() = getLinkAnnotations(0, length).map { substring(it.start, it.end) to it.item }

    @Test
    fun linksCoverTheirRunsAndOpen() {
        val bold = PLAIN.copy(bold = true)
        val url = "https://example.org/a"
        val text = text(plain("see "), TextRun.Text("the ", PLAIN, url), TextRun.Text("plots", bold, url), plain("."))
        assertEquals("see the plots.", text.text)
        val (covered, link) = text.linked().single()
        assertEquals("the plots", covered)
        assertEquals(url, (link as LinkAnnotation.Url).url)
        link.linkInteractionListener?.onClick(link)
        assertEquals(listOf(url), opened)
        // Bold inside the link.
        assertTrue(text.spanStyles.any { it.item.fontWeight == FontWeight.Bold && text.substring(it.start, it.end) == "plots" })
    }

    @Test
    fun aMentionShowsThePersonBoldIfUs() {
        val text = text(plain("ask "), TextRun.Mention("@ann:example.org", "Ann", PLAIN, you = true))
        val (covered, link) = text.linked().single()
        assertEquals("Ann", covered)
        val clickable = link as LinkAnnotation.Clickable
        assertEquals(PERSON + "@ann:example.org", clickable.tag)
        assertEquals(FontWeight.Bold, clickable.styles?.style?.fontWeight)
        clickable.linkInteractionListener?.onClick(clickable)
        assertEquals(listOf("@ann:example.org" to "Ann"), people)
    }

    @Test
    fun formulasAreInlineContentOnceTypesetElseLatexCode() {
        val formula = Formula("E=mc^2", display = false)
        val inlineTag = "androidx.compose.foundation.text.inlineContent"
        val typeset = text(plain("Energy "), TextRun.Math(formula, null), typeset = { it == formula })
        val inline = typeset.getStringAnnotations(inlineTag, 0, typeset.length).single()
        assertEquals(formulaId(formula), inline.item)
        assertEquals("E=mc^2", typeset.substring(inline.start, inline.end))

        val waiting = text(plain("Energy "), TextRun.Math(formula, null))
        assertEquals("Energy \$E=mc^2\$", waiting.text)
        assertTrue(waiting.getStringAnnotations(inlineTag, 0, waiting.length).isEmpty())
        assertTrue(waiting.spanStyles.any { it.item.fontFamily == FontFamily.Monospace && waiting.substring(it.start, it.end) == "\$E=mc^2\$" })
    }

    @Test
    fun aHiddenSpoilerIsTappedToShowTheSpoilers() {
        val text = text(plain("it was "), TextRun.Text("the butler", PLAIN.copy(spoiler = true), null))
        val (covered, link) = text.linked().single()
        assertEquals("the butler", covered)
        link.linkInteractionListener?.onClick(link)
        assertEquals(1, spoilers)
        val shown = annotated(listOf(TextRun.Text("the butler", PLAIN.copy(spoiler = true), null)), false, look.copy(spoilersShown = true), links) { false }
        assertTrue(shown.linked().isEmpty())
    }

    @Test
    fun editedFollowsTheLastParagraphOnly() {
        assertEquals("hi (edited)", text(plain("hi"), edited = true).text)
        val shown = TextBlock(BlockKind.Math(Formula("x", display = true)), quotes = 0u, lists = 0u, marker = null)
        assertTrue(editedInline(listOf(paragraph(plain("hi")))))
        assertFalse(editedInline(listOf(paragraph(plain("hi")), shown)))
        assertEquals(listOf(Formula("x", display = true)), formulas(listOf(paragraph(plain("hi")), shown)))
        assertEquals("\$\$x\$\$", fallback(Formula("x", display = true)))
    }
}
