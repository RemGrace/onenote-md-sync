#!/usr/bin/env python3
"""What the PAUSED sentinel is allowed to stop.

The file does two unrelated jobs and must not treat them the same. A human
hitting Pause means "stop syncing on a schedule" — it says nothing about
signing in, and if it also blocked sign-in the admin would have no way back
once the Microsoft session expired mid-pause. A smart lockout writes the same
file to mean "stop trying to sign in", and nothing may run through that:
retrying a locked-out account is what makes the lockout worse.

Every entry point (the admin, a scheduled run, a hand-started script) reads the
sentinel through here, so the two jobs can never drift apart between them.
"""
import json


def read_pause(path):
    """The sentinel as a dict, or None when runs are not paused.

    The admin writes JSON, but a file dropped in by hand must still count as a
    pause — its whole text becomes the reason.
    """
    try:
        raw = path.read_text().strip()
    except OSError:
        return None
    try:
        parsed = json.loads(raw)
    except ValueError:
        parsed = None
    return parsed if isinstance(parsed, dict) else {'reason': raw or 'paused'}


def is_lockout(paused):
    """True when Microsoft caused this pause, not a person."""
    return 'lockout' in str((paused or {}).get('reason', '')).lower()


def blocks(paused, by_hand=False):
    """Does this pause stop the run that is about to start?

    `by_hand` is an operator asking for this one run explicitly — Run now, or a
    re-login. That goes through a human pause, because the human pausing the
    schedule and the human asking for the run are the same person.
    """
    return paused is not None and (is_lockout(paused) or not by_hand)
