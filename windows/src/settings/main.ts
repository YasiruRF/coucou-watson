// Settings window — the place where anything that writes to disk is confirmed.
// Stage 2 covers the Claude Code hooks and the general preferences; API keys and
// integrations land here too in a later stage.

import "./settings.css";
import { Bridge, onEvent, type HookStatus, type HookPreview, type ModelInfo } from "../core/bridge";
import { DEFAULT_SETTINGS, type Settings } from "../core/state";
import { h, clear } from "../views/dom";

let settings: Settings = { ...DEFAULT_SETTINGS };
let version = "";

const root = document.getElementById("settings-root")!;

async function save() {
  await Bridge.saveSettings(settings);
}

// ── Reusable bits ─────────────────────────────────────────────────────────────

function toggle(on: boolean, onChange: (v: boolean) => void): HTMLElement {
  const el = h("button", { class: on ? "switch on" : "switch", "aria-pressed": on });
  el.addEventListener("click", () => {
    const next = !el.classList.contains("on");
    el.classList.toggle("on", next);
    onChange(next);
  });
  return el;
}

function statusDot(ok: boolean): HTMLElement {
  return h("i", { class: "dot", style: `background:${ok ? "#22c55e" : "#f4505e"}` });
}

function renderDiff(text: string): HTMLElement {
  const box = h("div", { class: "diff" });
  for (const line of text.split("\n")) {
    const cls = line.startsWith("+") ? "add" : line.startsWith("-") ? "del" : "ctx";
    box.append(h("div", { class: cls, text: line }));
  }
  return box;
}

// ── Hook sections (Claude Code, Antigravity) ────────────────────────────────
// Same three-step contract on both: preview the exact diff, back up, write only
// on an explicit click. `hookSection` is that flow; each caller only supplies
// its own wording and which Bridge calls to use.

interface HookSectionCopy {
  title: string;
  installed: string;
  notInstalled: string;
  relayMissing: string;
  installLabel: string;
  reinstallLabel: string;
  uninstallLabel: string;
  previewInstall: string;
  previewUninstall: string;
  doneText: (backup: string) => string;
}

interface HookSectionApi {
  status: () => Promise<HookStatus | null>;
  preview: (install: boolean) => Promise<HookPreview>;
  apply: (install: boolean, fingerprint: string) => Promise<string>;
}

function hookSection(copy: HookSectionCopy, api: HookSectionApi, status: HookStatus): HTMLElement {
  const body = h("div", { style: "display:flex;flex-direction:column;gap:12px" });
  const head = () => h("h2", {}, statusDot(status.installed), h("span", { text: copy.title }));
  const section = h("section", {}, head(), body);

  const rebuild = async () => {
    const fresh = await api.status();
    if (fresh) Object.assign(status, fresh);
    clear(body);
    draw();
    const oldHead = section.querySelector("h2")!;
    section.replaceChild(head(), oldHead);
  };

  function draw() {
    body.append(
      h("div", { class: "hint", text: status.installed ? copy.installed : copy.notInstalled }),
      h("div", { class: "row" },
        h("label", { text: "settings.json" }),
        h("span", { class: "path", text: status.settingsPath }),
      ),
      h("div", { class: "row" },
        h("label", { text: "Relay" }),
        h("span", { class: "path", text: status.hookPath }),
        statusDot(status.hookReady),
      ),
    );

    if (!status.hookReady) {
      body.append(h("div", { class: "notice warn", text: copy.relayMissing }));
    }

    const actions = h("div", { class: "row" });
    const install = h("button", {
      class: "primary",
      text: status.installed ? copy.reinstallLabel : copy.installLabel,
      onclick: () => showPreview(true),
    });
    // Writing hook commands that point at a relay which isn't there would give
    // every session a broken hook and nothing to show for it.
    if (!status.hookReady) {
      install.disabled = true;
      install.title = "The relay isn't installed yet.";
    }
    actions.append(install);
    if (status.installed) {
      actions.append(h("button", { class: "danger", text: copy.uninstallLabel, onclick: () => showPreview(false) }));
    }
    body.append(actions);
  }

  async function showPreview(install: boolean) {
    let preview;
    try {
      preview = await api.preview(install);
    } catch (err) {
      // An unreadable or invalid file stops here rather than being treated as
      // empty and written over.
      clear(body);
      body.append(
        h("div", { class: "notice err", text: String(err).replace(/^Error:\s*/, "") }),
        h("div", { class: "row" }, h("button", { text: "Back", onclick: () => { clear(body); draw(); } })),
      );
      return;
    }
    clear(body);
    body.append(
      h("div", { class: "hint", text: install ? copy.previewInstall : copy.previewUninstall }),
      renderDiff(preview.diff),
      h("div", { class: "row" }, h("span", { class: "path", text: `Backup → ${preview.backup}` })),
    );
    const confirm = h("button", {
      class: install ? "primary" : "danger",
      text: install ? "Back up and write" : "Back up and remove",
    });
    confirm.addEventListener("click", async () => {
      confirm.disabled = true;
      try {
        const backup = await api.apply(install, preview.fingerprint);
        clear(body);
        body.append(h("div", { class: "notice ok", text: copy.doneText(backup) }));
        window.setTimeout(() => void rebuild(), 2600);
      } catch (err) {
        confirm.disabled = false;
        body.append(h("div", { class: "notice err", text: `Could not write: ${String(err)}` }));
      }
    });
    body.append(h("div", { class: "row" }, confirm, h("button", { text: "Cancel", onclick: () => { clear(body); draw(); } })));
  }

  draw();
  return section;
}

function claudeSection(status: HookStatus): HTMLElement {
  return hookSection(
    {
      title: "Claude Code",
      installed: "Coucou is hooked into your Claude Code sessions. Tool calls, questions and permission requests show up in the island, and you can answer them there.",
      notInstalled: "Install the hooks to see your Claude Code sessions in the island and approve permissions without leaving what you are doing.",
      relayMissing: "coucou-hook.exe is not in place yet. Restart Coucou; if it still fails, build it with `cargo build -p coucou-hook`.",
      installLabel: "Install hooks…",
      reinstallLabel: "Reinstall hooks…",
      uninstallLabel: "Uninstall hooks…",
      previewInstall: "This is exactly what will change in your settings.json. Your own hooks are left untouched.",
      previewUninstall: "This removes Coucou's entries only. Your own hooks are left untouched.",
      doneText: (backup) => `Done. Previous settings saved as ${backup}. Open a new Claude Code session to pick the hooks up.`,
    },
    { status: Bridge.hooksStatus, preview: Bridge.hooksPreview, apply: Bridge.hooksApply },
    status,
  );
}

function agySection(status: HookStatus): HTMLElement {
  return hookSection(
    {
      title: "Antigravity",
      installed: "Coucou is hooked into Antigravity. Its sessions get their own pill in the island, next to Claude Code.",
      notInstalled: "Install the hooks to see Antigravity's sessions in the island as their own pill.",
      relayMissing: "coucou-hook.exe is not in place yet. Restart Coucou; if it still fails, build it with `cargo build -p coucou-hook`.",
      installLabel: "Install hooks…",
      reinstallLabel: "Reinstall hooks…",
      uninstallLabel: "Uninstall hooks…",
      previewInstall: "This is exactly what will change in ~/.gemini/config/hooks.json. Anything that isn't Coucou's is left untouched.",
      previewUninstall: "This removes Coucou's entries only. Anything that isn't Coucou's is left untouched.",
      doneText: (backup) => `Done. Previous file saved as ${backup}. Start a new Antigravity session to pick the hooks up.`,
    },
    { status: Bridge.agyHooksStatus, preview: Bridge.agyHooksPreview, apply: Bridge.agyHooksApply },
    status,
  );
}

// ── Chat section: Anthropic + Google AI keys, one model picker ─────────────────

const CLAUDE_MODELS: ModelInfo[] = [
  { id: "claude-opus-5", label: "Claude Opus 5" },
  { id: "claude-sonnet-5", label: "Claude Sonnet 5" },
  { id: "claude-haiku-4-5", label: "Claude Haiku 4.5" },
];

const DEFAULT_GEMINI_MODELS: ModelInfo[] = [
  { id: "gemini-2.5-flash", label: "Gemini 2.5 Flash (recommended — fast & cheap)" },
  { id: "gemini-2.5-pro", label: "Gemini 2.5 Pro (advanced reasoning)" },
  { id: "gemini-2.0-flash", label: "Gemini 2.0 Flash" },
  { id: "gemini-1.5-flash", label: "Gemini 1.5 Flash" },
  { id: "gemini-1.5-pro", label: "Gemini 1.5 Pro" },
];

/**
 * One provider's API key row: status dot, password field, save/remove, its own
 * feedback line. Anthropic and Google AI are two of these side by side.
 */
function apiKeyRow(opts: {
  key: string;
  placeholder: string;
  hasKey: boolean;
  onChange: () => void;
}): { dot: HTMLElement; row: HTMLElement; feedback: HTMLElement } {
  const dot = statusDot(opts.hasKey);
  const field = h("input", {
    type: "password",
    placeholder: opts.hasKey ? "••••••••••••  (stored)" : opts.placeholder,
    style: "flex:1 1 auto;min-width:0",
    autocomplete: "off",
    spellcheck: "false",
  }) as HTMLInputElement;
  const saveBtn = h("button", { class: "primary", text: "Save key" });
  const clearBtn = h("button", { class: "danger", text: "Remove", style: opts.hasKey ? "" : "display:none" });
  const feedback = h("div", {});

  async function refresh() {
    const present = (await Bridge.secretPresent(opts.key)) ?? false;
    dot.style.background = present ? "#22c55e" : "#f4505e";
    field.placeholder = present ? "••••••••••••  (stored)" : opts.placeholder;
    clearBtn.style.display = present ? "" : "none";
  }

  saveBtn.addEventListener("click", async () => {
    const value = field.value.trim();
    if (!value) return;
    clear(feedback);
    try {
      await Bridge.secretSet(opts.key, value);
      field.value = "";
      feedback.append(h("div", { class: "notice ok", text: "Saved. It never touches disk." }));
      await refresh();
      opts.onChange();
    } catch (err) {
      feedback.append(h("div", { class: "notice err", text: `Could not save: ${String(err)}` }));
    }
  });

  clearBtn.addEventListener("click", async () => {
    clear(feedback);
    try {
      await Bridge.secretClear(opts.key);
      feedback.append(h("div", { class: "notice ok", text: "Key removed." }));
      await refresh();
      opts.onChange();
    } catch (err) {
      feedback.append(h("div", { class: "notice err", text: `Could not remove: ${String(err)}` }));
    }
  });

  return { dot, row: h("div", { class: "row" }, field, saveBtn, clearBtn), feedback };
}

function apiSection(hasAnthropicKey: boolean, hasGoogleKey: boolean, initialGoogleModels: ModelInfo[]): HTMLElement {
  const modelSelect = h("select", {}) as HTMLSelectElement;
  let googleModels = initialGoogleModels;

  function rebuildModelOptions() {
    clear(modelSelect);
    const googleGroup = h("optgroup", { label: "Google Gemini (recommended)" });
    const list = googleModels.length > 0 ? googleModels : DEFAULT_GEMINI_MODELS;
    for (const m of list) googleGroup.append(h("option", { value: m.id, text: m.label }));
    modelSelect.append(googleGroup);

    const claudeGroup = h("optgroup", { label: "Claude (Anthropic)" });
    for (const m of CLAUDE_MODELS) claudeGroup.append(h("option", { value: m.id, text: m.label }));
    modelSelect.append(claudeGroup);

    // The saved model may belong to a provider with no key configured yet (or
    // whose list hasn't loaded): keep it selectable rather than silently losing it.
    const known = [...DEFAULT_GEMINI_MODELS, ...googleModels, ...CLAUDE_MODELS].some((m) => m.id === settings.model);
    if (!known) modelSelect.append(h("option", { value: settings.model, text: settings.model }));
    modelSelect.value = settings.model;
  }

  async function refreshGoogleModels() {
    googleModels = (await Bridge.googleModels()) ?? [];
    rebuildModelOptions();
  }

  const google = apiKeyRow({
    key: "google-ai-api-key",
    placeholder: "AIza...",
    hasKey: hasGoogleKey,
    onChange: () => void refreshGoogleModels(),
  });
  const claude = apiKeyRow({
    key: "anthropic-api-key",
    placeholder: "sk-ant-...",
    hasKey: hasAnthropicKey,
    onChange: () => {},
  });

  modelSelect.addEventListener("change", () => {
    settings.model = modelSelect.value;
    void save();
  });
  rebuildModelOptions();

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: "Chat & AI Models" })),
    h("div", { class: "row" }, google.dot, h("label", { text: "Google AI (Gemini) API key" })),
    google.row,
    google.feedback,
    h("div", { class: "hint", text: "Get a free Gemini API key from Google AI Studio (aistudio.google.com). GEMINI_API_KEY environment variable is also supported." }),
    h("div", { class: "row", style: "margin-top: 8px" }, h("label", { text: "Model" }), modelSelect),
    h("div", { class: "row", style: "margin-top: 14px" }, claude.dot, h("label", { text: "Claude API key (optional)" })),
    claude.row,
    claude.feedback,
  );
}

// ── Integrations section ──────────────────────────────────────────────────────

interface IntegrationDef {
  id: string;
  name: string;
  color: string;
  /** Credential Manager keys, in the order they are shown. */
  fields: { key: string; label: string; placeholder: string; secret: boolean }[];
}

const INTEGRATIONS: IntegrationDef[] = [
  { id: "integration_stripe", name: "Stripe", color: "#0570DE",
    fields: [{ key: "stripe-api-key", label: "Secret key", placeholder: "sk_live_…", secret: true }] },
  { id: "integration_github", name: "GitHub", color: "#F4505E",
    fields: [{ key: "github-token", label: "Token", placeholder: "ghp_…", secret: true }] },
  { id: "integration_vercel", name: "Vercel", color: "#7C5CFF",
    fields: [{ key: "vercel-token", label: "Token", placeholder: "…", secret: true }] },
  { id: "integration_n8n", name: "n8n", color: "#F29B38",
    fields: [
      { key: "n8n-url", label: "Instance URL", placeholder: "https://n8n.example.com", secret: false },
      { key: "n8n-api-key", label: "API key", placeholder: "…", secret: true },
    ] },
  { id: "integration_resend", name: "Resend", color: "#22C55E",
    fields: [{ key: "resend-api-key", label: "API key", placeholder: "re_…", secret: true }] },
  { id: "integration_notion", name: "Notion", color: "#8C8C8C",
    fields: [{ key: "notion-api-key", label: "Integration token", placeholder: "ntn_…", secret: true }] },
  { id: "integration_calcom", name: "Cal.com", color: "#C9956A",
    fields: [{ key: "calcom-api-key", label: "API key", placeholder: "cal_…", secret: true }] },
];

const MAX_ACTIVE = 4;

function integrationsSection(present: Record<string, boolean>): HTMLElement {
  const note = h("div", { class: "hint" });
  const list = h("div", { style: "display:flex;flex-direction:column;gap:14px" });

  function updateNote() {
    const used = settings.activeIntegrations.length;
    note.textContent = `Pick up to ${MAX_ACTIVE} pills to show next to Mochi — ${used}/${MAX_ACTIVE} in use. Keys are stored in the Windows Credential Manager, never on disk.`;
  }

  for (const def of INTEGRATIONS) {
    const active = settings.activeIntegrations.includes(def.id);
    const sw = h("button", { class: active ? "switch on" : "switch" });
    sw.addEventListener("click", () => {
      const on = settings.activeIntegrations.includes(def.id);
      if (on) {
        settings.activeIntegrations = settings.activeIntegrations.filter((x) => x !== def.id);
      } else {
        if (settings.activeIntegrations.length >= MAX_ACTIVE) return;
        settings.activeIntegrations = [...settings.activeIntegrations, def.id];
      }
      sw.classList.toggle("on", !on);
      updateNote();
      void save();
    });

    const rows = h("div", { style: "display:flex;flex-direction:column;gap:6px;flex:1 1 auto;min-width:0" });
    for (const field of def.fields) {
      const input = h("input", {
        type: field.secret ? "password" : "text",
        placeholder: present[field.key] ? "••••••••  (stored)" : field.placeholder,
        autocomplete: "off",
        spellcheck: "false",
        style: "flex:1 1 auto;min-width:0",
      }) as HTMLInputElement;
      const saveBtn = h("button", { text: "Save" });
      const dotEl = statusDot(present[field.key] ?? false);
      saveBtn.addEventListener("click", async () => {
        const value = input.value.trim();
        try {
          await Bridge.secretSet(field.key, value);
          present[field.key] = value.length > 0;
          input.value = "";
          input.placeholder = value ? "••••••••  (stored)" : field.placeholder;
          dotEl.style.background = value ? "#22c55e" : "#f4505e";
        } catch {
          dotEl.style.background = "#f5a524";
        }
      });
      rows.append(
        h("div", { class: "row" },
          h("label", { style: "min-width:104px", text: field.label }),
          input, saveBtn, dotEl,
        ),
      );
    }

    list.append(
      h("div", { style: "display:flex;gap:12px;align-items:flex-start" },
        h("div", { style: "display:flex;align-items:center;gap:8px;min-width:132px;padding-top:4px" },
          sw,
          h("i", { class: "dot", style: `background:${def.color}` }),
          h("span", { style: "font-size:12.5px", text: def.name }),
        ),
        rows,
      ),
    );
  }

  updateNote();
  return h("section", {}, h("h2", {}, h("span", { text: "Integrations" })), note, list);
}

// ── General section ───────────────────────────────────────────────────────────

function generalSection(): HTMLElement {
  const volume = h("input", {
    type: "range", min: "0", max: "0.2", step: "0.005",
    value: String(settings.soundVolume),
  }) as HTMLInputElement;
  volume.addEventListener("input", () => {
    settings.soundVolume = Number(volume.value);
    void save();
  });

  const autoClose = h("input", {
    type: "number", min: "5", max: "120", step: "1",
    value: String(Math.round(settings.autoCloseInterval)),
    style: "width:72px",
  }) as HTMLInputElement;
  autoClose.addEventListener("change", () => {
    settings.autoCloseInterval = Math.max(5, Math.min(120, Number(autoClose.value) || 15));
    autoClose.value = String(settings.autoCloseInterval);
    void save();
  });

  const screen = h("select", {}) as HTMLSelectElement;
  screen.append(
    h("option", { value: "primary", text: "Main display" }),
    h("option", { value: "cursor", text: "Display under the cursor" }),
  );
  screen.value = settings.screen;
  screen.addEventListener("change", () => {
    settings.screen = screen.value as Settings["screen"];
    void save();
  });

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: "General" })),
    h("div", { class: "row" },
      h("label", { text: "Sound" }),
      toggle(settings.soundEnabled, (v) => { settings.soundEnabled = v; void save(); }),
      volume,
    ),
    h("div", { class: "row" },
      h("label", { text: "Auto-close" }),
      autoClose,
      h("span", { class: "hint", text: "seconds after you leave the island" }),
    ),
    h("div", { class: "row" },
      h("label", { text: "Island lives on" }),
      screen,
    ),
    h("div", { class: "row" },
      h("label", { text: "Launch at startup" }),
      toggle(settings.autostart, (v) => { settings.autostart = v; void save(); }),
    ),
  );
}

// ── Boot ──────────────────────────────────────────────────────────────────────

async function main() {
  const boot = await Bridge.boot();
  if (boot) {
    settings = { ...settings, ...boot.settings };
    version = boot.version;
  }
  const status = (await Bridge.hooksStatus()) ?? {
    installed: false, settingsPath: "", hookPath: "", hookReady: false,
  };
  const agyStatus = (await Bridge.agyHooksStatus()) ?? {
    installed: false, settingsPath: "", hookPath: "", hookReady: false,
  };

  const hasAnthropicKey = (await Bridge.secretPresent("anthropic-api-key")) ?? false;
  const hasGoogleKey = (await Bridge.secretPresent("google-ai-api-key")) ?? false;
  const googleModels = hasGoogleKey ? ((await Bridge.googleModels()) ?? []) : [];

  const keys = [
    "stripe-api-key", "github-token", "vercel-token",
    "n8n-url", "n8n-api-key", "resend-api-key", "notion-api-key", "calcom-api-key",
  ];
  const present: Record<string, boolean> = {};
  for (const k of keys) present[k] = (await Bridge.secretPresent(k)) ?? false;

  clear(root);
  root.append(
    h("h1", {}, h("span", { text: "Coucou" }), h("span", { class: "version", text: version })),
    claudeSection(status),
    agySection(agyStatus),
    apiSection(hasAnthropicKey, hasGoogleKey, googleModels),
    integrationsSection(present),
    generalSection(),
    h("div", {
      class: "hint",
      text: "No telemetry. Network requests only go to the services you configure yourself.",
    }),
  );

  void onEvent<Settings>("settings-changed", (s) => {
    settings = { ...settings, ...s };
  });
}

void main();
