package net.xfbs.specto

import java.io.File
import java.util.Locale
import javax.xml.parsers.DocumentBuilderFactory

/**
 * The app's English texts, read from `res/values/strings.xml`: unit tests run without Android's
 * resources, and this way they still check what the reader gets to see, placeholders included.
 */
object EnglishTexts : Texts {
    /** The texts by name. Gradle runs unit tests in the project's directory. */
    private val byName: Map<String, String> = run {
        val file = File("res/values/strings.xml")
        val strings = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(
            file,
        ).getElementsByTagName("string")
        (0 until strings.length).associate { i ->
            val string = strings.item(i)
            // What Android's own reading of the file does to a text.
            string.attributes.getNamedItem(
                "name",
            ).nodeValue to string.textContent.replace("\\'", "'")
        }
    }

    /** The names of the texts by their number in the generated `R.string`. */
    private val names: Map<Int, String> =
        R.string::class.java.fields.associate { it.getInt(null) to it.name }

    override fun get(id: Int, vararg args: Any): String {
        val text = byName.getValue(names.getValue(id))
        return if (args.isEmpty()) text else String.format(Locale.ROOT, text, *args)
    }
}
