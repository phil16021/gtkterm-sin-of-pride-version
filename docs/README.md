# Documentation

- [ROADMAP.md](ROADMAP.md) — audit findings and the phased implementation, test and debugging plan (2026-10-17).
- [analysis/](analysis/) — earlier code-analysis write-ups (course assignment, Nov 2025).
  - **Caution:** several of them describe features as working that were never wired into the
    application (TCP/UDP ports, protocol decoders, logging pipeline, XMODEM, toolbar, shortcuts),
    and `FIXES_APPLIED.md` claims a warning-free build that did not hold. Treat them as historical;
    the roadmap is authoritative.
  - [analysis/demo/](analysis/demo/) — standalone valgrind/gprof demo programs; they do not touch GTKTerm code.
