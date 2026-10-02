# Handover — Coucou Windows session (Claude Code → Antigravity IDE)

Repo: `D:\Praxis\coucou\coucou`, branch `main`, remote `origin` →
`https://github.com/YasiruRF/coucou-watson.git` (just re-pointed from the
original `Louis-CFM/coucou` fork). Working tree is clean on commit
`8137846`.

## What this session did, in order

1. **Repo setup.** Re-pointed `origin`, verified it matches local `main`.
   Uninstalled the old build (`D:\Praxis\coucou\coucou.exe`, its registry
   entries, autostart, shortcut) and wiped `windows/node_modules`, `dist`,
   `release`, `target`. Fresh `npm install`, confirmed a clean `npm run
   tauri dev` boots.
2. **Fixed a real bug**, found by screenshot: the overview ticker (the
   scrolling step list under the Claude Code pill) froze mid-transition and
   showed two step texts stacked on one line.
3. **Added three things the user asked for**, listed below.
4. **Committed** (`8137846`) once everything typechecked, compiled and the
   unit tests passed.

## Changes, by area

### 1. Ticker stall (bug fix)
- Root cause: `island.ts`'s frame loop (`requestAnimationFrame`) stops
  running whenever nothing it recognizes is animating, to keep CPU at 0%
  when idle. A view mid-animation (the ticker's step transition) wasn't on
  that list, so a burst of PreToolUse/PostToolUse events could leave the
  ticker's internal clock started but never ticked again.
- Fix: `ViewHost` gained an optional `busy(): boolean` (`views/views.ts`),
  the overview view implements it from `Ticker.animating`, and
  `island.ts`'s busy-check now includes `this.views.get(State.view)?.busy?.()`.
- Also fixed: `Ticker.sync()` compared `task.stepIndex` to know whether a
  new step arrived, but `AgentTask.steps` is capped at 20 entries
  (`state.ts`) — once capped, the index stops moving and the ticker thought
  nothing new had happened. Added `AgentTask.stepCount` (a running total,
  bumped in `state.ts#appendStep`) and rewrote `Ticker.sync()` to key off
  that instead.
- **Verified live** in the browser pane by injecting `window.__coucou`
  (a dev-only global exposed from `main.ts`, gated by `import.meta.env.DEV`
  so it's stripped from production builds) and driving a burst of steps
  past the 20-step cap. Confirmed the ticker no longer overlaps and the
  loop no longer stalls.

### 2. Floating ball + click-outside
User asked for: clicking outside the open island folds it back to the
notch; instead of the island just vanishing when idle, Mochi floats as a
small draggable ball that auto-returns to the notch.

- **New FSM state `"ball"`** in `island/fsm.ts`. `isIdle()` (wired from
  `island.ts` to `!State.hasActiveSession`, a new getter in `state.ts`)
  decides whether an about-to-hide notch becomes `hidden` or `ball`.
  `ballReturnDelay` (20s) brings it back to the notch on its own; once back,
  `restInNotch` keeps it from immediately hiding or re-ballifying — it just
  sits in the notch until something happens (`reveal()`/`forceHome()` clear
  that flag).
- **Rust side** (`src-tauri/src/island.rs`): the ball lives in a second,
  smaller OS window (`BALL_W`/`BALL_H` = 104 logical px, vs the 720×320
  panel) so it can sit anywhere on screen, not just glued to the top edge.
  Dragging is done on the **existing 60Hz cursor-poll thread**
  (`spawn_cursor_poll`), not the page: a press on the ball
  (`ball_drag_start` command) starts a `BallDrag` the poll thread advances
  every tick by moving the window with `set_position`, and releases it with
  a `ball-released` event carrying whether it was a drag or a tap (`moved:
  bool`, tap-slop = 4 physical px). This was deliberate — handing the drag
  to the page would mean the page loses the pointer the instant the window
  itself moves out from under it.
- **Click-outside**: same poll thread now also detects a press whose cursor
  is off the island shape (`on_island` was already computed for
  click-through) and emits `outside-click`. `island.ts#onOutsideClick()`
  folds the island back to the notch unless `State.isPinned` (an approval
  card waiting for an answer) — same rule as Escape already had.
- New Tauri commands: `set_ball`, `ball_drag_start`. New window events:
  `outside-click`, `ball-released`.
- `core/layout.ts`: `IslandMode` gained `"ball"`; `BALL_SIZE` (88, the drawn
  circle) and `BALL_WINDOW` (104, the OS window) constants;
  `islandSize`/`botPosition` branches for it.
- **Verified live**: drove `island.fsm` directly in the browser pane
  (`forceHome()` → `onOutsideClick()` → folds to notch; `forcePetit()` with
  no active session → auto-ballifies after the (shortened, for the test)
  timer → auto-returns and stays in the notch). Confirmed the DOM ends up
  88×88, fully round (`border-radius: 44px`), with Mochi centered inside —
  screenshot taken and matches expectations. **Not yet tested**: actually
  dragging the Rust-side window with a real mouse (the browser pane can't
  drive native window drags), and the `BallDrag`/tap-vs-drag logic beyond
  unit-level reasoning — there's no automated test for `island.rs` at all
  (none existed before this session either; it's not a unit-testable
  module as written, it's all OS calls).

### 3. Antigravity IDE support
User clarified: they use **Antigravity IDE**, not a standalone CLI. Good
news — the hook config path (`~/.gemini/config/hooks.json`) is shared by
both the IDE and CLI installs on this machine (confirmed by listing
`~/.gemini/` — both `antigravity` and `antigravity-ide` subfolders exist
under the same `.gemini/config/`), so nothing here is CLI-specific.

- `windows/hook/src/main.rs` (the `coucou-hook.exe` relay): added
  `normalize_event()` and `normalize_tool_fields()`, ported **exactly** from
  `NotchBuddy/Sources/App/HookServer.swift`'s Python relay (the macOS
  app already supports Antigravity + Gemini CLI). Translates
  `PreInvocation`→`UserPromptSubmit`, `PostInvocation`/`AfterTool`→
  `PostToolUse`, `BeforeTool`→`PreToolUse`, etc., and flattens
  `toolCall.{name,args}` → `tool_name`/`tool_input`, `conversationId` →
  `session_id`. Falls back to `workspacePaths[0]` for `cwd` when Antigravity
  doesn't send one. Three new unit tests, all passing.
- **New `src-tauri/src/agy.rs`**: the Antigravity hooks.json installer,
  same preview → dated backup → fingerprinted write contract as the
  existing Claude Code one (`hooks.rs`). I pulled `parse_settings`,
  `pretty`, `stamp`, `fingerprint`, `unified_diff` up to `pub(crate)` in
  `hooks.rs` and extracted a shared `commit()` step so `agy.rs` reuses
  them rather than re-implementing. Coucou's entries live under one
  top-level `"coucou"` key in `hooks.json` — same layout the Mac app
  writes, so a `hooks.json` with both platforms is readable by either.
  4 unit tests, all passing.
- `island/hooks.ts`: Antigravity (and Gemini CLI) now get a proper label
  and the SPEC.md color (`#E879F9` / `#8AB4F8`) instead of a hashed
  fallback color, via a `KNOWN_AGENTS` map. Unlike other third-party
  agents, their pill **stays and resets to idle** after a session ends
  (`clearAgent()`) instead of being removed — matches how the Claude Code
  pill behaves, and makes sense for an agent you've explicitly named.
- Settings UI: `settings/main.ts`'s Claude-only hook section got pulled out
  into a generic `hookSection(copy, api, status)` (the diff-preview-backup-
  write flow was identical, just the wording and three Bridge calls
  differed), and `agySection()` reuses it against the three new
  `agy_hooks_*` Tauri commands.
- **Not tested live** — no Antigravity session was triggered against the
  running app this session (would need Antigravity IDE actually configured
  to call the relay, which nobody has installed yet). Compile + unit tests
  only. **This is the main thing to verify by hand**: install the hooks
  from Settings → Antigravity, open an Antigravity IDE session, and watch
  for the pill.

### 4. Google AI (Gemini) chat
User asked: allow Google AI as a chat provider, dropdown shows all
available Gemini models so the cheaper ones can be picked.

- **New `src-tauri/src/google.rs`**: chat client against Google's
  OpenAI-compatible endpoint (`.../v1beta/openai/chat/completions`), same
  approach as `ClaudeService.chatOpenAICompatible` on macOS. **Shares the
  existing `Chat` history** with `claude.rs` (its three private methods
  `is_empty`/`push`/`pop`/`snapshot` became `pub(crate)`) rather than
  keeping a second conversation — Anthropic's API accepts a plain string
  for `content` just as readily as a block array, so a conversation that
  starts on Claude and switches to Gemini (or back) keeps its context
  either way.
- Model listing: `google::fetch_models(key)` hits
  `.../v1beta/openai/models`, strips the `models/` prefix, filters out
  non-chat families (embed/imagen/veo/aqa/tts/audio/live — same list as
  macOS), and **sorts cheapest-first** by a `cost_rank()` heuristic
  (flash-lite < flash < pro < ultra < other) since the API doesn't return
  pricing. New Tauri command `google_models()` — reads the stored key
  itself server-side; the key never crosses into JS.
- `chat_send` in `lib.rs` routes to `google::send` when
  `claude::is_google_model(&model)` (i.e. the model id starts with
  `"gemini"`), else the existing `claude::send`. No new Settings field for
  "provider" — the model id itself is the routing key, kept it to the
  existing single `settings.model: String`.
- Settings UI: the old Anthropic-only "Claude" API section is now a
  "Chat" section with **two key rows** (`apiKeyRow()`, factored out since
  Anthropic and Google AI needed the identical save/remove/status-dot
  UI) and **one `<select>`** with `<optgroup>` for Claude vs Google models.
  Saving/removing the Google key re-fetches the model list live.
- 2 unit tests (content-flattening, cost ranking), both passing.
- **Not tested live** — no Google AI key was available in this session.
  **Verify by hand**: Settings → paste a Google AI Studio key → confirm the
  model dropdown populates with real Gemini models, cheapest first → send a
  chat message → confirm a reply comes back → switch back to a Claude model
  mid-conversation and confirm it still has context.

## Build state
- `npx tsc --noEmit` — clean.
- `cargo check` (full `coucou` crate) — clean, 0 warnings.
- `cargo test` (full crate + `coucou-hook`) — 20/20 passing.
- `npm run tauri dev` — boots and runs (confirmed via background task output;
  not click-tested with a real mouse this session beyond the browser-pane
  JS injection described above).

## Known gaps / what to check first
1. **Antigravity IDE, for real.** Install the hooks from Settings, open a
   real session, confirm the pill appears, labels and ticker steps read
   correctly, and that `Stop`/`SessionEnd` actually reset the pill to idle
   rather than removing it.
2. **Ball dragging with a real mouse.** The FSM transitions and DOM shape
   were verified; actually grabbing and dragging the ball window, and the
   tap-vs-drag threshold (4 physical px), were not — the browser pane can
   script DOM/JS but can't drive a native OS window drag.
3. **Google AI chat with a real key.** Model fetch, a real chat turn, and
   mid-conversation model switching are implemented and unit-tested at the
   boundary (content flattening, cost ranking) but not run end-to-end.
4. `windows/src/main.ts` has a `window.__coucou` dev-only escape hatch
   (`if (import.meta.env.DEV) ...`) used for the ticker test. It's
   build-stripped in production (Vite replaces `import.meta.env.DEV` with
   `false` and dead-code-eliminates the branch) but it's new — worth a
   second look if a leaner diff is wanted.
5. Full list of touched/added files: `git show --stat 8137846` (21 files;
   new: `windows/src-tauri/src/agy.rs`, `windows/src-tauri/src/google.rs`).
   Full rationale for every change is in the commit message
   (`git log -1 --format=%B`).

## Everything else from the original ask
The user also asked for a general "run through all systems and suggest
improvements" — **not done this session**, ran out of scope/budget after
the four items above. Worth a pass: `docs/SPEC.md` and `docs/AGENTS.md`
are the behavior spec (in French) to compare the Windows port against;
`windows/README.md` lists known platform differences from macOS.
