import Swal from 'sweetalert2';

/** Asks before something irreversible happens, and resolves to whether it may.
 *
 *  The only place sweetalert2 is configured. Every guarded action asks the same
 *  way — same red confirm on the right, same Cancel focused — because a dialog
 *  that looks different each time is a dialog people learn to click through.
 *  Callers say what is at stake and what the button should read; nothing else.
 *
 *  Cancel holds the focus on purpose: the keyboard's default answer to "delete
 *  this" should be no. Dismissing — Cancel, Escape, a click outside — resolves
 *  false and sends nothing.
 *
 *  The scheme is read off Mantine's own attribute at the moment the dialog
 *  opens, not captured once: the app runs on `auto`, so Mantine resolves the
 *  attribute to a literal light/dark and rewrites it when the OS flips. A value
 *  read at import time would show a white dialog over a dark page for the rest
 *  of the session. sweetalert2 paints its own popup and inherits none of
 *  Mantine's variables, so it has to be told rather than left to follow.
 */
export async function confirmed({ title, text, action }) {
  const { isConfirmed } = await Swal.fire({
    // titleText, not title: sweetalert2 renders `title` as HTML. Nothing
    // interpolates into a title today, but `text` two lines down already carries a
    // Microsoft display name the tenant controls, and the next caller has no way to
    // tell from here which of these two fields is a markup sink. Neither is now.
    titleText: title,
    text,
    icon: 'warning',
    theme: document.documentElement.dataset.mantineColorScheme === 'dark' ? 'dark' : 'light',
    showCancelButton: true,
    confirmButtonText: action,
    cancelButtonText: 'Cancel',
    // Mantine's own red, so the destructive button is the same red as the
    // destructive buttons the page already shows
    confirmButtonColor: 'var(--mantine-color-red-filled)',
    // Cancel first, confirm last — the order the rest of the app puts them in
    reverseButtons: true,
    focusCancel: true,
  });
  return isConfirmed;
}
