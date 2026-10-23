# Review

Clients review a report in their browser; their comments come back into the layout. The
portal is a separate service ([designcraft-review](https://github.com/ChrisVerbick/designcraft-review)),
and the app talks to it with the commands below. Everything is a command, so the control
channel, scripts and MCP reach it all; the Review panel (Window › Editorial › Review) is the
same commands with buttons. The desktop app and the command-line tool (`designcraft-cli run`,
`script`, `mcp`) carry the connection to the service; a job can publish right after its export.
With automation roots set, `review.publish` is refused: it exports through a temporary folder
outside them.

## Set up once

The service's URL and its owner token live in Preferences (`prefs.set {reviewServer, reviewToken}`;
the panel has fields for both). An edition names its own service (`[review] server` in
`edition.toml`, [editions.md](editions.md)), the default when the preference is empty. They are kept in the preferences file on this computer: anyone
using this account can read the token, which can do everything the owner can on the service.
A keychain entry is the planned upgrade. The token never goes into an edition build.

## Publish

`review.publish {project?, scale?}` exports the review bundle ([review-bundle.md](review-bundle.md))
and uploads it as a new version of the document. The first time, name the project (a client,
usually: `review.projects` lists them, `review.createProject {name, client?}` makes one). After
that the service knows which project the document belongs to; a different project is refused
and named, and `review.detach {}` is the way to start a review of a copy for another client.

A share link for the project (`review.link {mode?: invite|public, password?, expiresDays?}`)
is shown once; the service keeps only its hash. Invite-only links need the reviewer's address
on the project's member list (the service's owner API); public links take anyone with the link
and, when set, the password.

## Sync and act

`review.sync {}` fetches the comments. `review.comments {}` lists them with `anchored`: where
each comment's text is in the document now, found at its bytes, moved (found by its quote, with
the words around it telling repeats apart), or orphaned (gone; the service is told so once).

- `review.goTo {id}` selects the comment's text where it is now.
- `review.addNote {id}` puts the comment in the text as an editorial note by "Reviewer (Review)",
  at the end of its text (at the story's start when the text is gone, or when it sits in a table
  cell or footnote, which notes don't go in).
- `review.apply {id}` turns a replace or strike suggestion into a tracked change at its text:
  Track Changes then accepts or rejects it, as any change. An orphaned suggestion is never
  applied.
- `review.resolve {id, on?}` and `review.reply {id, body}` act on the service; replies are signed
  with Preferences' `userName`.

The panel filters by status, reviewer and round and shows replies under their comment. Network
calls run on the interface thread: a publish of a large report pauses the app for the seconds
the upload takes.

## Rounds and decisions

A round is what the reviewers look at together: every document of the project at its latest
version, a due date, and a lock when the owner closes it. `review.openRound {name?, days?, note?}`
opens one (`due` in seconds instead of `days`); the share page lists the round with its documents
and, for signed-in reviewers, Approve and Request changes (with a note) on each: one decision per
reviewer and document, changeable while the round is open. `review.lockRound {id?}` locks the
latest round: its versions take no more comments or decisions, and stay readable with their
comments. The next round shows whatever was published since. `review.rounds {}` lists the rounds
as last synced, with their decisions; the panel shows the current round, the decisions on this
document and the comments by round (a comment carries the round it was made in).

The day before a round is due, the service mails every member who hasn't decided on all of its
documents, once. Members are those invited by address. A guest of a public link typed their own
name, so their decision is shown as "(guest)" and they get no reminder: a round whose approvals
must count as sign-off runs on an invite link.

`review.invite {members: [{email, name?}]}` puts reviewers on the project's member list and
mails each an invite link (a fresh link each time; a link can be revoked on the service).
