package net.xfbs.specto

import android.graphics.Rect
import android.view.ActionMode
import android.view.Menu
import android.view.MenuItem
import android.view.View

/**
 * The floating menu beside selected text: the system's own (the one text fields show), with the
 * entries the page view wants.
 *
 * Compose's `TextToolbar` shows the same menu but knows only cut, copy, paste and select all, and
 * the page view needs "Share".
 *
 * [view] is the view the menu belongs to. The places given to [show] are in its coordinates.
 */
class TextMenu(private val view: View) {
    /** An entry: what it says and what choosing it does. */
    class Item(val title: String, val action: () -> Unit)

    private var mode: ActionMode? = null
    private var items: List<Item> = emptyList()
    private val beside = Rect()

    private val callback = object : ActionMode.Callback2() {
        override fun onCreateActionMode(mode: ActionMode, menu: Menu): Boolean {
            for ((index, item) in items.withIndex()) {
                menu.add(Menu.NONE, index, index, item.title).setShowAsAction(MenuItem.SHOW_AS_ACTION_IF_ROOM)
            }
            return true
        }

        override fun onPrepareActionMode(mode: ActionMode, menu: Menu) = false

        override fun onActionItemClicked(mode: ActionMode, item: MenuItem): Boolean {
            val chosen = items.getOrNull(item.itemId) ?: return false
            mode.finish()
            chosen.action()
            return true
        }

        override fun onDestroyActionMode(mode: ActionMode) {
            if (this@TextMenu.mode === mode) this@TextMenu.mode = null
        }

        override fun onGetContentRect(mode: ActionMode, view: View, outRect: Rect) = outRect.set(beside)
    }

    /**
     * Shows the menu with [items] beside the rectangle given (pixels), replacing one that is
     * showing.
     */
    fun show(left: Float, top: Float, right: Float, bottom: Float, items: List<Item>) {
        hide()
        this.items = items
        beside.set(left.toInt(), top.toInt(), right.toInt(), bottom.toInt())
        mode = view.startActionMode(callback, ActionMode.TYPE_FLOATING)
    }

    fun hide() {
        mode?.finish()
        mode = null
    }
}
