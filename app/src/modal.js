/**
 * One dialog implementation, because there were four.
 *
 * Every dialog here carried `role="dialog"` and `aria-modal="true"` — a
 * promise to assistive technology that the rest of the page is unreachable —
 * and none of them kept it. Tab walked out of "Remove device?" into the tab
 * bar behind the backdrop, where a screen-reader user could activate the very
 * thing the dialog was asking about. Closing dropped focus to `<body>`, so
 * whoever opened it with the keyboard lost their place. Issue #75.
 *
 * `aria-modal` is a claim, not a mechanism. `inert` is the mechanism: it takes
 * a subtree out of the tab order, out of hit-testing and out of the
 * accessibility tree in one attribute, which is exactly what "modal" means.
 * The trap below is still needed because `inert` cannot apply to an ancestor
 * of the dialog, and the dialog's own siblings inside `<body>` are what has to
 * be neutralised.
 *
 * Not `<dialog>` with `showModal()`. It would give the trap and the top layer
 * for free, but these panels are already styled as full-screen flex backdrops
 * on five platforms, and `::backdrop` plus the UA's own centring would have to
 * be undone on every one of them. The behaviour is what was missing, not the
 * markup.
 */

/**
 * The dialog currently open, if any.
 *
 * A module-level single because there is one user and one screen. Two open at
 * once was reachable — through the Tab escape this file closes — and left the
 * first dialog's promise unresolved for ever, with the button that opened it
 * stuck reading "…".
 */
let current = null;

/**
 * What can be tabbed to, in document order.
 *
 * Visibility is checked, not just the attributes: the ask dialog hides its
 * text field for a yes/no question, and a trap that cycled through a hidden
 * input would strand focus somewhere invisible.
 */
function focusable(root) {
  const candidates = root.querySelectorAll(
    'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
  );
  return [...candidates].filter(
    (el) => !el.hidden && el.offsetParent !== null && !el.closest("[hidden]"),
  );
}

/** How long to keep trying to hand focus back, and how often. */
const RESTORE_DEADLINE = 300;
const RESTORE_POLL = 16;

/**
 * Put focus back where it was before the dialog opened.
 *
 * Not synchronously, and not once. Two things can make the opener refuse
 * focus at the moment the dialog closes, and both clear on their own:
 *
 * - It is usually a button still mid-action. `withButton` disables it for the
 *   duration, a disabled element cannot take focus, and it is re-enabled as
 *   the promise this dialog just resolved unwinds.
 * - `inert` was lifted from its ancestors one line above this call, and at
 *   least one Chrome does not treat the subtree as focusable again within the
 *   same task.
 *
 * This is why a single `setTimeout(0)` was not enough: it passed here on
 * every run and failed on another machine, connected, enabled and a real
 * button, with a longer wait afterwards changing nothing — because the one
 * retry had already fired and given up before the wait began. Which of the
 * two causes it was is still unknown, and that is the point: the condition
 * being waited for is observable, so poll it rather than bet a duration
 * against it. `child.rs` reaches for the same shape for the same reason.
 *
 * Bounded, so a genuinely unfocusable opener stops rather than spins, and
 * abandoned the moment another dialog opens, so this cannot pull focus out
 * from under it.
 */
function restoreFocus(opener) {
  const take = () => {
    if (!opener || !opener.isConnected || !opener.focus) return true;
    opener.focus();
    return document.activeElement === opener;
  };
  if (take()) return;
  const until = Date.now() + RESTORE_DEADLINE;
  const again = () => {
    if (current || take() || Date.now() > until) return;
    setTimeout(again, RESTORE_POLL);
  };
  setTimeout(again, 0);
}

/**
 * Open `box` as a modal dialog.
 *
 * Returns a `close` function, or `null` if a dialog is already open — the
 * caller is expected to treat that as a cancel rather than wait for something
 * that will never resolve.
 *
 * `onClose` runs before focus is restored, so a caller can clear the panel's
 * contents without the user watching it happen in a focused field.
 */
export function openModal(box, { focus = null, onClose = null } = {}) {
  if (current) return null;

  // Captured before the dialog takes focus, which is the only moment it is
  // still available. Restoring to `<body>` is what a dropped focus looks like.
  const opener = document.activeElement;
  const inerted = [];
  for (const sibling of document.body.children) {
    if (sibling !== box && !sibling.inert) {
      sibling.inert = true;
      inerted.push(sibling);
    }
  }

  box.hidden = false;

  const first = focus ?? focusable(box)[0] ?? box;
  first.focus();
  if (first.select && first.value) first.select();

  const onKey = (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
      return;
    }
    if (e.key !== "Tab") return;
    // The trap. `inert` on the siblings already stops focus leaving the
    // dialog, but without this the last element tabs to the browser's own
    // chrome and back, which reads as focus having vanished.
    const stops = focusable(box);
    if (stops.length === 0) {
      e.preventDefault();
      return;
    }
    const edge = e.shiftKey ? stops[0] : stops[stops.length - 1];
    if (document.activeElement === edge || !box.contains(document.activeElement)) {
      e.preventDefault();
      (e.shiftKey ? stops[stops.length - 1] : stops[0]).focus();
    }
  };

  // Only the backdrop itself, so a click inside the panel does not cancel.
  const onClick = (e) => {
    if (e.target === box) close();
  };

  function close() {
    if (current !== close) return;
    current = null;
    document.removeEventListener("keydown", onKey);
    box.removeEventListener("click", onClick);
    box.hidden = true;
    for (const sibling of inerted) sibling.inert = false;
    if (onClose) onClose();
    restoreFocus(opener);
  }

  current = close;
  document.addEventListener("keydown", onKey);
  box.addEventListener("click", onClick);
  return close;
}
