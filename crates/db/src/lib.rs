//! THE single SQLite layer. Device ships SQLite 3.12.2 — every statement
//! here must be 3.12-safe (no HAVING without GROUP BY, no modern SQL).
//! Holds: gametime sessions, battery log, Dude state, RA offline cache index.
