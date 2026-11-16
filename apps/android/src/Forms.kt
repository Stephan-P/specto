package net.xfbs.specto

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import uniffi.specto_ffi.FieldTap

/** Identifies the field that a text field of a form is typed into, to UI tests. */
const val FIELD_TEXT_TAG = "field-text"

/** How many lines of a text field of several lines show at once. */
private const val FIELD_LINES = 6

/**
 * Asks for what goes into the field of the form that was tapped ([PdfViewState.askedField]): the
 * text of a text field, or what to choose of the options of a menu or a list. Shows nothing while no
 * field is asked about.
 */
@Composable
fun FieldDialog(state: PdfViewState) {
    when (val asked = state.askedField) {
        is FieldTap.Text -> FieldTextDialog(asked, state::setFieldText, state::endField)

        is FieldTap.Choose ->
            FieldChoiceDialog(asked, state::chooseOption, state::setFieldText, state::endField)

        is FieldTap.ChooseSeveral ->
            FieldSeveralDialog(asked, state::chooseOptions, state::endField)

        else -> {}
    }
}

/**
 * Asks for the text of the text field [asked], starting from what it holds.
 *
 * "Done" hands the text to [onDone]. So does the keyboard's own button in a field of one line,
 * where Enter can't mean a new line. [onDismiss] leaves the field as it is.
 */
@Composable
private fun FieldTextDialog(
    asked: FieldTap.Text,
    onDone: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    // The cursor starts behind what is there, to go on typing.
    var value by remember(asked) {
        mutableStateOf(TextFieldValue(asked.text, TextRange(asked.text.length)))
    }
    val most = asked.maxLength?.toInt()
    val focus = remember { FocusRequester() }
    LaunchedEffect(asked) { focus.requestFocus() }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(asked.label ?: stringResource(R.string.field_text_title)) },
        text = {
            OutlinedTextField(
                value = value,
                onValueChange = { new ->
                    value = if (most != null && new.text.length > most) {
                        new.copy(text = new.text.take(most))
                    } else {
                        new
                    }
                },
                modifier = Modifier.fillMaxWidth().focusRequester(focus).testTag(FIELD_TEXT_TAG),
                supportingText = most?.let {
                    { Text(stringResource(R.string.field_length, value.text.length, it)) }
                },
                singleLine = !asked.multiline,
                maxLines = if (asked.multiline) FIELD_LINES else 1,
                visualTransformation = if (asked.password) {
                    PasswordVisualTransformation()
                } else {
                    VisualTransformation.None
                },
                keyboardOptions = KeyboardOptions(
                    keyboardType = if (asked.password) KeyboardType.Password else KeyboardType.Text,
                    imeAction = if (asked.multiline) ImeAction.Default else ImeAction.Done,
                ),
                keyboardActions = KeyboardActions(onDone = { onDone(value.text) }),
            )
        },
        confirmButton = {
            TextButton(onClick = { onDone(value.text) }) {
                Text(stringResource(R.string.field_done))
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel)) }
        },
    )
}

/**
 * Shows what the menu or list [asked] offers, with what it holds marked.
 *
 * A tap on a row hands its index to [onChoose]. A menu that takes text of one's own has a line to
 * type it above the rows, and "Done" hands that text to [onTyped]. [onDismiss] leaves the field as
 * it is.
 */
@Composable
private fun FieldChoiceDialog(
    asked: FieldTap.Choose,
    onChoose: (Int) -> Unit,
    onTyped: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    val chosen = asked.chosen?.toInt()
    // A long list starts where the chosen row is.
    val rows = rememberLazyListState(initialFirstVisibleItemIndex = chosen ?: 0)
    val typed = asked.typed
    var own by remember(asked) { mutableStateOf(typed.orEmpty()) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(asked.label ?: stringResource(R.string.field_choice_title)) },
        text = {
            Column {
                if (typed != null) {
                    OutlinedTextField(
                        value = own,
                        onValueChange = { own = it },
                        modifier = Modifier.fillMaxWidth().testTag(FIELD_TEXT_TAG),
                        label = { Text(stringResource(R.string.field_own_text)) },
                        singleLine = true,
                        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                        keyboardActions = KeyboardActions(onDone = { onTyped(own) }),
                    )
                }
                LazyColumn(state = rows) {
                    items(asked.options.size) { index ->
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .selectable(
                                    selected = index == chosen,
                                    role = Role.RadioButton,
                                    onClick = { onChoose(index) },
                                )
                                .padding(vertical = 12.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            RadioButton(selected = index == chosen, onClick = null)
                            Text(asked.options[index], Modifier.padding(start = 16.dp))
                        }
                    }
                }
            }
        },
        confirmButton = {
            if (typed != null) {
                TextButton(onClick = { onTyped(own) }) {
                    Text(stringResource(R.string.field_done))
                }
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel)) }
        },
    )
}

/**
 * Shows what the list [asked] offers, which takes several choices, with a tick on each row it
 * holds.
 *
 * A tap on a row ticks or unticks it. "Done" hands the ticked rows to [onDone]. [onDismiss] leaves
 * the field as it is.
 */
@Composable
private fun FieldSeveralDialog(
    asked: FieldTap.ChooseSeveral,
    onDone: (Set<Int>) -> Unit,
    onDismiss: () -> Unit,
) {
    var chosen by remember(asked) { mutableStateOf(asked.chosen.map { it.toInt() }.toSet()) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(asked.label ?: stringResource(R.string.field_choice_title)) },
        text = {
            LazyColumn {
                items(asked.options.size) { index ->
                    val ticked = index in chosen
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .toggleable(
                                value = ticked,
                                role = Role.Checkbox,
                                onValueChange = {
                                    chosen = if (it) chosen + index else chosen - index
                                },
                            )
                            .padding(vertical = 12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Checkbox(checked = ticked, onCheckedChange = null)
                        Text(asked.options[index], Modifier.padding(start = 16.dp))
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = { onDone(chosen) }) { Text(stringResource(R.string.field_done)) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel)) }
        },
    )
}
