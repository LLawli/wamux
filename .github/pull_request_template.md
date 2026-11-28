Closes #

## The problem

<!-- What is wrong or missing, as a user or an edge sees it. Quote the error or the log line. -->

## What changes

<!-- The change, and why this way when the issue offered others. Numbers when a choice was measured. -->

## What changes on the contract

<!-- "No wire change." Or each .proto, config key, default, CLI or store change, and whether it breaks.
     A breaking change links its upgrade guide under docs/upgrading/. -->

## Verified in the code

<!-- What you read end to end: the path you are fixing or relying on, in wamux or in whatsapp-rust. -->

## Inferred, not verified

<!-- What you concluded from behaviour or logs without reading the code that does it. "Nothing" is a fine answer. -->

## Validation

- `scripts/ci.sh`: <!-- the last line, e.g. CI PASSED -->
- Live run: <!-- what was exercised against a real account, or "none". No phone numbers or secrets. -->

## Checklist

- [ ] `CHANGELOG.md` `[Unreleased]` has an entry, if the change is visible to a user, an edge or an operator.
- [ ] A breaking wire, config or store change has an upgrade guide under `docs/upgrading/`, listed in its `README.md`.
- [ ] Nothing here is edge policy (retries, timeouts, fallbacks, recipient rewriting, filtering, auth).
