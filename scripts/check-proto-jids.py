#!/usr/bin/env python3
"""#72 / #120: every field of the socket contract that carries a jid is the
`Jid` message, never a bare `string`.

Usage: scripts/check-proto-jids.py [--root DIR]   (DIR defaults to the repo)

Why: the contract used two representations for the same thing. About 50
fields held a jid as a `string` beside 18 that used `Jid`, so a consumer could
not tell from the schema which string was a jid, and the same value changed
type between two RPCs.

A `string` field fails when its name ends in `jid` or `jids`, or names a jid by
its role (`chat`, `sender`, `participant(s)`, `recipient(s)`, `voter(s)`,
`from`, `lid`, `pn`, ...). Only `crates/wamux-proto/proto/*.proto` is read:
`proto/store/` is the on-disk blob format, not the contract.

PENDING names the fields #121 and #122 still have to migrate, each with its
issue. A pending field that is no longer a `string` jid fails too, so the list
only shrinks. The scan must read at least MIN_SCANNED files, so a scan that
reads nothing cannot pass.
"""
import argparse
import re
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
PROTO_DIR = "crates/wamux-proto/proto"
MIN_SCANNED = 8
ROLES = {
    "chat", "chats", "sender", "sender_alt", "recipient", "recipients", "recipient_alt",
    "participant", "participants", "voter", "voters", "creator", "poll_creator", "from",
    "lid", "pn", "group", "newsletter", "mention", "mentions",
}
PENDING: dict[str, str] = {
    "contacts.proto:CheckOnWhatsAppRequest.jids": "#121",
    "contacts.proto:CheckResult.jid": "#121",
    "contacts.proto:JidRequest.jid": "#121",
    "contacts.proto:SubscribePresenceRequest.jid": "#121",
    "contacts.proto:LidPnMapping.lid": "#121",
    "contacts.proto:LidPnMapping.pn": "#121",
    "contacts.proto:ResolveLidPnRequest.jids": "#121",
    "events.proto:InboundMessage.chat": "#122",
    "events.proto:InboundMessage.sender": "#122",
    "events.proto:InboundMessage.sender_alt": "#122",
    "events.proto:InboundMessage.recipient_alt": "#122",
    "events.proto:ReceiptEvent.chat": "#122",
    "events.proto:ReceiptEvent.sender": "#122",
    "events.proto:UndecryptableEvent.chat": "#122",
    "events.proto:UndecryptableEvent.sender": "#122",
    "events.proto:PresenceUpdate.jid": "#122",
    "events.proto:PresenceUpdate.chat": "#122",
    "events.proto:GroupUpdate.group_jid": "#122",
    "events.proto:PushNameUpdate.jid": "#122",
    "events.proto:ContactUpdate.jid": "#122",
    "events.proto:AppStateUpdate.chat": "#122",
    "events.proto:FavoritesChanged.chats": "#122",
    "events.proto:NewsletterLiveUpdate.newsletter_jid": "#122",
    "events.proto:CallEvent.from": "#122",
    "events.proto:ServerAckEvent.from": "#122",
    "groups.proto:CreateGroupRequest.participants": "#121",
    "groups.proto:GroupJidResponse.group_jid": "#121",
    "groups.proto:ParticipantsRequest.group_jid": "#121",
    "groups.proto:ParticipantsRequest.participants": "#121",
    "groups.proto:GroupTextRequest.group_jid": "#121",
    "groups.proto:GroupRef.group_jid": "#121",
    "groups.proto:GroupSummary.jid": "#121",
    "groups.proto:GroupToggleRequest.group_jid": "#121",
    "groups.proto:GroupEphemeralRequest.group_jid": "#121",
    "groups.proto:SetGroupPhotoRequest.group_jid": "#121",
    "groups.proto:ParticipantChange.jid": "#121",
    "newsletters.proto:Newsletter.jid": "#121",
    "newsletters.proto:GetNewsletterMessagesRequest.jid": "#121",
    "newsletters.proto:SendNewsletterPollVoteRequest.jid": "#121",
    "newsletters.proto:GetMyNewsletterAddOnsRequest.jid": "#121",
}
FIELD = re.compile(r"^\s*(?:repeated\s+|optional\s+)?string\s+(\w+)\s*=\s*\d+")
BLOCK = re.compile(r"^\s*(?:message|enum|oneof|service)\s+(\w+)\s*\{")


def is_jid_name(name: str) -> bool:
    return name.endswith("jid") or name.endswith("jids") or name in ROLES


def string_jid_fields(path: Path, rel: str) -> list[tuple[str, int]]:
    """(`<file>:<Message>.<field>`, line) of each string field named for a jid."""
    hits = []
    stack: list[str] = []
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        code = line.split("//", 1)[0]
        block = BLOCK.match(code)
        for opened in range(code.count("{")):
            # The block's own brace carries its name; any other (an option
            # literal) keeps the enclosing message as the owner.
            stack.append(block.group(1) if block and opened == 0 else (stack[-1] if stack else "?"))
        field = FIELD.match(code)
        if field and is_jid_name(field.group(1)):
            owner = stack[-1] if stack else "?"
            hits.append((f"{rel}:{owner}.{field.group(1)}", number))
        del stack[max(0, len(stack) - code.count("}")) :]
    return hits


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="no string jid fields in the contract (#72)")
    parser.add_argument("--root", type=Path, default=REPO)
    args = parser.parse_args(argv)
    problems: list[str] = []
    found: set[str] = set()
    files = sorted((args.root / PROTO_DIR).glob("*.proto"))
    for path in files:
        rel = path.relative_to(args.root / PROTO_DIR).as_posix()
        for name, number in string_jid_fields(path, rel):
            found.add(name)
            if name not in PENDING:
                problems.append(f"{PROTO_DIR}/{rel.split(':')[0]}:{number}: {name.split(':')[1]} is a string jid")
    problems += [f"{name} is no longer a string jid: remove it from PENDING" for name in sorted(PENDING) if name not in found]
    if len(files) < MIN_SCANNED:
        problems.append(f"scanned {len(files)} files under {PROTO_DIR}, expected at least {MIN_SCANNED}: the layout moved or the scan broke")
    if problems:
        print("proto jid check FAILED (a jid field is the Jid message, #72):")
        print("\n".join(f"  - {p}" for p in problems))
        return 1
    print(f"no string jid field in {len(files)} proto files ({len(PENDING)} pending: {', '.join(sorted(set(PENDING.values())))})")
    return 0


if __name__ == "__main__":
    import sys

    sys.exit(main(sys.argv[1:]))
