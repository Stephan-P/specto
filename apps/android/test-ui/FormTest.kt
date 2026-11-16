package net.xfbs.specto

import androidx.compose.ui.test.TouchInjectionScope
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.click
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTouchInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Filling in the form of `form.pdf`: a tap ticks its box, and asks for the text of a text field or
 * for one of the options of its menu.
 *
 * What is filled in is drawn with the page, where tests don't see it. They go by what the app says:
 * that the document is edited, what there is to undo, and what a field holds when it is tapped
 * again. That the value is in the saved file is tested in Rust (`filling_in_a_form`).
 */
@RunWith(AndroidJUnit4::class)
class FormTest : AppTest() {
    private fun openForm() = open("form.pdf", shows = "1 / 2")

    // The middles of the fields (see `make-fixtures.py`).
    private fun TouchInjectionScope.name() = pagePoint(300f, fromTop = 141f)
    private fun TouchInjectionScope.code() = pagePoint(200f, fromTop = 181f)
    private fun TouchInjectionScope.agree() = pagePoint(160f, fromTop = 222f)
    private fun TouchInjectionScope.country() = pagePoint(250f, fromTop = 261f)
    private fun TouchInjectionScope.color() = pagePoint(250f, fromTop = 301f)
    private fun TouchInjectionScope.days() = pagePoint(250f, fromTop = 362f)

    private val field get() = compose.onNodeWithTag(FIELD_TEXT_TAG)

    private fun assertEdited(edited: Boolean) =
        compose.onAllNodesWithText("Edited").assertCountEquals(if (edited) 1 else 0)

    @Test
    fun aTapTicksABox() {
        openForm()
        page.performTouchInput { click(agree()) }
        waitForText("Edited")
        // It did nothing else: the bars stay.
        compose.onNodeWithContentDescription("More").assertIsDisplayed()
        menu("Undo fill in field")
        assertEdited(false)
        menu("Redo fill in field")
        assertEdited(true)
    }

    @Test
    fun textIsTypedIntoAField() {
        openForm()
        page.performTouchInput { click(name()) }
        // Under the name the form has for it.
        waitForText("Your name")
        field.performTextInput("Ada")
        compose.onNodeWithText("Done").performClick()
        waitForText("Edited")

        // It holds that now. Leaving it as it is changes nothing.
        page.performTouchInput { click(name()) }
        waitForText("Your name")
        field.assertTextContains("Ada")
        field.performTextInput(" Lovelace")
        compose.onNodeWithText("Cancel").performClick()
        waitUntilGone("Your name")
        menu("Undo fill in field")
        assertEdited(false)
    }

    @Test
    fun aFieldTakesOnlyAsMuchAsItSays() {
        openForm()
        page.performTouchInput { click(code()) }
        // The form has no name for this one.
        waitForText("Fill in field")
        field.performTextInput("123456")
        field.assertTextContains("1234")
        compose.onNodeWithText("4 / 4").assertIsDisplayed()
        compose.onNodeWithText("Done").performClick()
        waitForText("Edited")
    }

    @Test
    fun aMenuOffersWhatThereIsToChoose() {
        openForm()
        page.performTouchInput { click(country()) }
        waitForText("Choose")
        compose.onNodeWithText("Germany").assertIsSelected()
        compose.onNodeWithText("France").assertIsNotSelected().performClick()
        waitForText("Edited")

        page.performTouchInput { click(country()) }
        waitForText("Choose")
        compose.onNodeWithText("France").assertIsSelected()
        compose.onNodeWithText("Cancel").performClick()
        waitUntilGone("Choose")
        menu("Undo fill in field")
        assertEdited(false)
    }

    @Test
    fun aMenuTakesTextOfOnesOwn() {
        openForm()
        page.performTouchInput { click(color()) }
        waitForText("Or type your own")
        compose.onNodeWithText("Red").assertIsNotSelected()
        field.performTextInput("Teal")
        compose.onNodeWithText("Done").performClick()
        waitForText("Edited")

        // It holds that now, and still offers its own.
        page.performTouchInput { click(color()) }
        waitForText("Or type your own")
        field.assertTextContains("Teal")
        compose.onNodeWithText("Green").performClick()
        waitUntilGone("Or type your own")
        page.performTouchInput { click(color()) }
        waitForText("Or type your own")
        field.assertTextContains("Green")
    }

    @Test
    fun aListTakesSeveralChoices() {
        openForm()
        page.performTouchInput { click(days()) }
        waitForText("Choose")
        compose.onNodeWithText("Mon").assertIsOff().performClick()
        compose.onNodeWithText("Wed").performClick()
        compose.onNodeWithText("Done").performClick()
        waitForText("Edited")

        page.performTouchInput { click(days()) }
        waitForText("Choose")
        compose.onNodeWithText("Mon").assertIsOn()
        compose.onNodeWithText("Tue").assertIsOff()
        compose.onNodeWithText("Wed").assertIsOn()
        compose.onNodeWithText("Cancel").performClick()
    }

    @Test
    fun theFormIsResetAndItsFieldsHighlighted() {
        openForm()
        page.performTouchInput { click(agree()) }
        waitForText("Edited")
        // Everything goes back to what the form starts with, as one step.
        menu("Reset form")
        menu("Undo reset form")
        menu("Undo fill in field")
        assertEdited(false)
        menu("Redo fill in field")
        menu("Redo reset form")
        page.performTouchInput { click(name()) }
        waitForText("Your name")
        field.assertTextContains("Nobody")
        compose.onNodeWithText("Cancel").performClick()

        // Highlighting is no edit, and the menu says whether it is on.
        menu("Highlight form fields")
        compose.onNodeWithContentDescription("More").performClick()
        compose.onNodeWithText("Highlight form fields").assertIsDisplayed()
    }

    @Test
    fun aDocumentWithoutAFormOffersNothingForOne() {
        open("pages.pdf", shows = "1 / 12")
        compose.onNodeWithContentDescription("More").performClick()
        compose.onNodeWithText("Go to page").assertIsDisplayed()
        compose.onAllNodesWithText("Reset form").assertCountEquals(0)
    }
}
