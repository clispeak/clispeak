# 54. The tray's left click opens the window; the menu is on the right

**Status:** Accepted.

`show_menu_on_left_click(false)` moved the menu onto the secondary button and
nothing was put in its place, so a left click on the tray icon did nothing at
all. The window was reachable only through a menu someone had to know to
right-click for — on the platform where the tray is the primary way back to a
window that closing has hidden. Reported from the machine rather than found by
reading: the code says exactly what it does, and nothing about it looks wrong.

Left click now calls the same `reveal` as "Show voicecast" and `voicecast
show`, so there is one answer to "bring the window back" rather than three.

**Why the menu is not on the left as well.** Both on one button means a click
that wanted the window gets a menu instead, and the window is what people ask
for. Quitting is deliberate and belongs a step further away — the same
reasoning that made closing the window hide it rather than quit.

**Why on release rather than press.** A button still down may yet become a
drag, and raising the window on the press moves it under a pointer that is
travelling away from it.

**Cost.** The menu is now genuinely hidden from anyone who does not think to
right-click. That is the trade: the discoverable gesture does the common
thing, and the menu holds what is rare.
