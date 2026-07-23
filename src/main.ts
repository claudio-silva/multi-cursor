import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

interface Environment {
  id: string;
  name: string;
  createdAt: string;
}

interface Account {
  id: string;
  envId: string;
  name: string;
  email?: string | null;
  updatedAt: string;
  pendingLogin: boolean;
}

interface ActiveSelection {
  envId?: string | null;
  accountId?: string | null;
}

interface AppConfig {
  environments: Environment[];
  accounts: Account[];
  active: ActiveSelection;
  cursorAppPath: string;
}

interface AppState {
  config: AppConfig;
  cursorRunning: boolean;
  rootDir: string;
}

interface ListStateResult {
  state: AppState;
  capturedEmail?: string | null;
}

type PromptResult = string | null;

interface TextPromptOptions {
  title: string;
  initial?: string;
  confirmLabel?: string;
  checkboxLabel?: string;
  checkboxDefault?: boolean;
}

interface TextPromptResult {
  value: string;
  checked: boolean;
}

interface CopyProgressEvent {
  percent: number;
  label: string;
}

const els = {
  envSelect: document.querySelector("#env-select") as HTMLSelectElement,
  envDisk: document.querySelector("#env-disk") as HTMLElement,
  accountList: document.querySelector("#account-list") as HTMLUListElement,
  status: document.querySelector("#status") as HTMLElement,
  error: document.querySelector("#error") as HTMLElement,
  success: document.querySelector("#success") as HTMLElement,
  hint: document.querySelector("#hint") as HTMLElement,
  progress: document.querySelector("#progress") as HTMLElement,
  progressLabel: document.querySelector("#progress-label") as HTMLElement,
  progressBar: document.querySelector("#progress-bar") as HTMLElement,
  launch: document.querySelector("#launch") as HTMLButtonElement,
  envNew: document.querySelector("#env-new") as HTMLButtonElement,
  envRename: document.querySelector("#env-rename") as HTMLButtonElement,
  envDelete: document.querySelector("#env-delete") as HTMLButtonElement,
  acctNew: document.querySelector("#acct-new") as HTMLButtonElement,
  acctDelete: document.querySelector("#acct-delete") as HTMLButtonElement,
  modal: document.querySelector("#modal") as HTMLElement,
  modalTitle: document.querySelector("#modal-title") as HTMLElement,
  modalMessage: document.querySelector("#modal-message") as HTMLElement,
  modalLink: document.querySelector("#modal-link") as HTMLAnchorElement,
  modalInput: document.querySelector("#modal-input") as HTMLInputElement,
  modalCheckWrap: document.querySelector("#modal-check-wrap") as HTMLElement,
  modalCheck: document.querySelector("#modal-check") as HTMLInputElement,
  modalCheckLabel: document.querySelector("#modal-check-label") as HTMLElement,
  modalCancel: document.querySelector("#modal-cancel") as HTMLButtonElement,
  modalConfirm: document.querySelector("#modal-confirm") as HTMLButtonElement,
};

interface UpdateCheckResult {
  updateAvailable: boolean;
  currentVersion: string;
  latestVersion: string;
  releaseUrl: string;
  message: string;
}

let state: AppState | null = null;
let selectedEnvId: string | null = null;
let selectedAccountId: string | null = null;
/** User picked an account in the list; do not clobber from polling/refresh. */
let userPickedAccount = false;
let modalResolver: ((value: PromptResult) => void) | null = null;
/** Captured before modal fields are cleared (checkbox would otherwise always be false). */
let lastModalChecked = false;
let flashUntil = 0;
let busy = false;
let busyLabel: string | null = null;
let diskUsageEnvId: string | null = null;
let diskUsageInFlight = false;
/** Env id to measure after the in-flight `du` finishes (rapid dropdown switches). */
let diskUsageQueued: string | null = null;

function showError(message: string | null) {
  if (!message) {
    els.error.classList.add("hidden");
    els.error.textContent = "";
    return;
  }
  els.success.classList.add("hidden");
  els.error.textContent = message;
  els.error.classList.remove("hidden");
  flashUntil = Date.now() + 8000;
}

function showSuccess(message: string | null, opts?: { sticky?: boolean }) {
  if (!message) {
    els.success.classList.add("hidden");
    els.success.textContent = "";
    flashUntil = 0;
    return;
  }
  els.error.classList.add("hidden");
  els.success.textContent = message;
  els.success.classList.remove("hidden");
  flashUntil = opts?.sticky ? Number.POSITIVE_INFINITY : Date.now() + 8000;
}

function showHint(message: string | null) {
  if (!message) {
    els.hint.classList.add("hidden");
    els.hint.textContent = "";
    return;
  }
  els.hint.textContent = message;
  els.hint.classList.remove("hidden");
}

function setProgress(percent: number | null, label?: string) {
  if (percent === null) {
    els.progress.classList.add("hidden");
    els.progressBar.style.width = "0%";
    els.progressLabel.textContent = "";
    els.progress.removeAttribute("aria-valuenow");
    return;
  }
  const p = Math.max(0, Math.min(100, Math.round(percent)));
  els.progress.classList.remove("hidden");
  els.progressBar.style.width = `${p}%`;
  els.progressLabel.textContent = label ? `${label} ${p}%` : `Copying… ${p}%`;
  els.progress.setAttribute("aria-valuenow", String(p));
}

function hideModalLink() {
  els.modalLink.classList.add("hidden");
  els.modalLink.removeAttribute("href");
  els.modalLink.textContent = "";
}

function closeModal(value: PromptResult) {
  // Read checkbox BEFORE clearing — askTextWithCheckbox needs this value.
  lastModalChecked = els.modalCheck.checked;
  els.modal.classList.add("hidden");
  els.modalInput.value = "";
  els.modalCheck.checked = false;
  els.modalCheckWrap.classList.add("hidden");
  hideModalLink();
  els.modalCancel.classList.remove("hidden");
  const resolve = modalResolver;
  modalResolver = null;
  // Defer past paint so “Restart” can hide the dialog before any IPC work.
  window.setTimeout(() => resolve?.(value), 0);
}

/** Wait until the browser has painted pending DOM updates (WKWebView needs a real timer). */
async function yieldToPaint(): Promise<void> {
  await new Promise<void>((resolve) => {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => resolve());
    });
  });
  await new Promise((r) => setTimeout(r, 80));
}

/** Show “Cursor is closing…”, paint, then quit (poll; force if needed). */
async function quitCursorWithUi(): Promise<void> {
  busy = true;
  busyLabel = "Closing…";
  showError(null);
  showSuccess("Cursor is closing…", { sticky: true });
  render();
  await yieldToPaint();

  await call<void>("quit_cursor_cmd");
  let closed = await waitForCursorState(false, 12_000);
  if (!closed) {
    await call<void>("force_quit_cursor_cmd");
    closed = await waitForCursorState(false, 8_000);
  }
  if (!closed) {
    throw "Cursor did not quit in time. Quit it manually and try again.";
  }
  if (state) state.cursorRunning = false;
}

function askText(title: string, initial = "", confirmLabel = "OK"): Promise<PromptResult> {
  return new Promise((resolve) => {
    modalResolver = resolve;
    els.modalTitle.textContent = title;
    els.modalMessage.classList.add("hidden");
    els.modalMessage.textContent = "";
    hideModalLink();
    els.modalInput.classList.remove("hidden");
    els.modalInput.value = initial;
    els.modalCheckWrap.classList.add("hidden");
    els.modalCancel.classList.remove("hidden");
    els.modalConfirm.textContent = confirmLabel;
    els.modalConfirm.classList.remove("danger");
    els.modal.classList.remove("hidden");
    requestAnimationFrame(() => {
      els.modalInput.focus();
      els.modalInput.select();
    });
  });
}

function askTextWithCheckbox(opts: TextPromptOptions): Promise<TextPromptResult | null> {
  return new Promise((resolve) => {
    modalResolver = (value) => {
      if (!value) {
        resolve(null);
        return;
      }
      // lastModalChecked is set in closeModal before the checkbox is cleared.
      resolve({ value, checked: lastModalChecked });
    };
    els.modalTitle.textContent = opts.title;
    els.modalMessage.classList.add("hidden");
    els.modalMessage.textContent = "";
    hideModalLink();
    els.modalInput.classList.remove("hidden");
    els.modalInput.value = opts.initial ?? "";
    els.modalCheckWrap.classList.remove("hidden");
    els.modalCheckLabel.textContent = opts.checkboxLabel ?? "";
    els.modalCheck.checked = Boolean(opts.checkboxDefault);
    els.modalCancel.classList.remove("hidden");
    els.modalConfirm.textContent = opts.confirmLabel ?? "OK";
    els.modalConfirm.classList.remove("danger");
    els.modal.classList.remove("hidden");
    requestAnimationFrame(() => {
      els.modalInput.focus();
      els.modalInput.select();
    });
  });
}

function askConfirm(
  title: string,
  message: string,
  confirmLabel = "OK",
  danger = false,
): Promise<boolean> {
  return new Promise((resolve) => {
    modalResolver = (value) => resolve(value === "ok");
    els.modalTitle.textContent = title;
    els.modalMessage.textContent = message;
    els.modalMessage.classList.remove("hidden");
    hideModalLink();
    els.modalInput.classList.add("hidden");
    els.modalCheckWrap.classList.add("hidden");
    els.modalCancel.classList.remove("hidden");
    els.modalConfirm.textContent = confirmLabel;
    els.modalConfirm.classList.toggle("danger", danger);
    els.modal.classList.remove("hidden");
    requestAnimationFrame(() => els.modalConfirm.focus());
  });
}

function showMessageDialog(
  title: string,
  message: string,
  linkUrl?: string | null,
): Promise<void> {
  return new Promise((resolve) => {
    modalResolver = () => resolve();
    els.modalTitle.textContent = title;
    els.modalMessage.textContent = message;
    els.modalMessage.classList.remove("hidden");
    if (linkUrl) {
      els.modalLink.href = linkUrl;
      els.modalLink.textContent = linkUrl;
      els.modalLink.classList.remove("hidden");
    } else {
      hideModalLink();
    }
    els.modalInput.classList.add("hidden");
    els.modalCheckWrap.classList.add("hidden");
    els.modalCancel.classList.add("hidden");
    els.modalConfirm.textContent = "OK";
    els.modalConfirm.classList.remove("danger");
    els.modal.classList.remove("hidden");
    requestAnimationFrame(() => els.modalConfirm.focus());
  });
}

async function checkForUpdates() {
  try {
    const result = await call<UpdateCheckResult>("check_for_updates");
    await showMessageDialog(
      result.updateAvailable ? "Update available" : "Check for Updates",
      result.message,
      result.updateAvailable ? result.releaseUrl : null,
    );
  } catch {
    // call() already surfaces the error in the UI
  }
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (err) {
    const message = typeof err === "string" ? err : String(err);
    showError(message);
    throw err;
  }
}

function accountsForEnv(envId: string | null): Account[] {
  if (!state || !envId) return [];
  return state.config.accounts.filter((a) => a.envId === envId);
}

function isActiveSelectionRunning(): boolean {
  if (!state?.cursorRunning || !selectedEnvId || !selectedAccountId) return false;
  return (
    state.config.active.envId === selectedEnvId &&
    state.config.active.accountId === selectedAccountId
  );
}

function accountDisplayName(account: Account): string {
  return account.name || account.email || "Account";
}

/** Label for confirm dialogs — includes email when it distinguishes the account. */
function accountConfirmLabel(account: Account): string {
  const name = account.name?.trim();
  const email = account.email?.trim();
  if (name && email && name !== email) return `${name} (${email})`;
  return name || email || "Account";
}

function accountInitials(account: Account): string {
  if (account.pendingLogin) return "?";
  const source = accountDisplayName(account).trim();
  if (!source) return "?";
  if (source.includes("@")) {
    const local = source.split("@")[0] ?? source;
    const parts = local.split(/[._+\-\s]+/).filter(Boolean);
    if (parts.length >= 2) {
      return `${parts[0]![0] ?? ""}${parts[1]![0] ?? ""}`.toUpperCase();
    }
    return local.slice(0, 2).toUpperCase();
  }
  const parts = source.split(/\s+/).filter(Boolean);
  if (parts.length >= 2) {
    const first = [...(parts[0] ?? "")][0] ?? "";
    const last = [...(parts[parts.length - 1] ?? "")][0] ?? "";
    return `${first}${last}`.toUpperCase();
  }
  return [...source].slice(0, 2).join("").toUpperCase();
}

const AVATAR_COLORS = [
  "#6b5cad",
  "#3d6ea8",
  "#2f7d6d",
  "#8a5a3c",
  "#4a6fa5",
  "#5c6bc0",
  "#3d7a8c",
];

function avatarColor(seed: string): string {
  let hash = 0;
  for (let i = 0; i < seed.length; i++) {
    hash = (hash * 31 + seed.charCodeAt(i)) >>> 0;
  }
  return AVATAR_COLORS[hash % AVATAR_COLORS.length]!;
}

function formatDiskUsage(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  const mb = bytes / (1024 * 1024);
  if (mb < 1024) return `${Math.max(1, Math.round(mb))} MB`;
  const gb = mb / 1024;
  return `${gb >= 10 ? gb.toFixed(0) : gb.toFixed(1)} GB`;
}

async function refreshEnvDiskUsage(force = false) {
  if (!selectedEnvId) {
    diskUsageEnvId = null;
    diskUsageQueued = null;
    els.envDisk.textContent = "Disk usage: —";
    return;
  }
  if (!force && diskUsageEnvId === selectedEnvId) return;
  if (diskUsageInFlight) {
    diskUsageQueued = selectedEnvId;
    return;
  }
  diskUsageInFlight = true;
  const envId = selectedEnvId;
  if (diskUsageEnvId !== envId) {
    els.envDisk.textContent = "Disk usage: …";
  }
  try {
    const bytes = await invoke<number>("environment_disk_usage", { envId });
    if (selectedEnvId === envId) {
      diskUsageEnvId = envId;
      els.envDisk.textContent = `Disk usage: ${formatDiskUsage(bytes)}`;
    }
  } catch {
    if (selectedEnvId === envId) {
      els.envDisk.textContent = "Disk usage: —";
    }
  } finally {
    diskUsageInFlight = false;
    const queued = diskUsageQueued;
    diskUsageQueued = null;
    if (queued && selectedEnvId === queued && diskUsageEnvId !== queued) {
      void refreshEnvDiskUsage(true);
    }
  }
}

function render() {
  if (!state) return;
  const { config, cursorRunning } = state;

  els.status.textContent = cursorRunning ? "Cursor running" : "Cursor idle";
  els.status.classList.toggle("running", cursorRunning);
  els.status.classList.toggle("idle", !cursorRunning);

  if (!selectedEnvId || !config.environments.some((e) => e.id === selectedEnvId)) {
    selectedEnvId = config.active.envId ?? config.environments[0]?.id ?? null;
  }

  els.envSelect.innerHTML = "";
  if (config.environments.length === 0) {
    const opt = document.createElement("option");
    opt.value = "";
    opt.textContent = "No environments";
    els.envSelect.appendChild(opt);
    els.envSelect.disabled = true;
  } else {
    els.envSelect.disabled = busy;
    for (const env of config.environments) {
      const opt = document.createElement("option");
      opt.value = env.id;
      const isCurrent = env.id === config.active.envId;
      opt.textContent = isCurrent ? `${env.name} (current)` : env.name;
      if (env.id === selectedEnvId) opt.selected = true;
      els.envSelect.appendChild(opt);
    }
  }

  const accounts = accountsForEnv(selectedEnvId);
  if (!selectedAccountId || !accounts.some((a) => a.id === selectedAccountId)) {
    selectedAccountId =
      (config.active.envId === selectedEnvId ? config.active.accountId : null) ??
      accounts[0]?.id ??
      null;
  }

  els.accountList.innerHTML = "";
  for (const account of accounts) {
    const li = document.createElement("li");
    li.dataset.id = account.id;
    if (account.id === selectedAccountId) li.classList.add("selected");

    const avatar = document.createElement("div");
    avatar.className = "avatar";
    avatar.textContent = accountInitials(account);
    avatar.style.background = avatarColor(account.id || accountDisplayName(account));
    li.appendChild(avatar);

    const body = document.createElement("div");
    body.className = "body";

    const name = document.createElement("div");
    name.className = "name";
    name.textContent = accountDisplayName(account);
    body.appendChild(name);

    const meta = document.createElement("div");
    meta.className = "meta";
    if (account.pendingLogin) {
      meta.textContent = "Sign in inside Cursor — email will appear here";
    } else if (account.email) {
      meta.textContent = account.email;
    }
    if (meta.textContent) body.appendChild(meta);
    li.appendChild(body);

    if (account.pendingLogin) {
      const badge = document.createElement("span");
      badge.className = "badge";
      badge.textContent = "waiting";
      li.appendChild(badge);
    } else if (
      account.id === config.active.accountId &&
      account.envId === config.active.envId
    ) {
      const badge = document.createElement("span");
      badge.className = "badge current";
      badge.textContent = "current";
      li.appendChild(badge);
    }

    li.addEventListener("click", () => {
      if (busy) return;
      selectedAccountId = account.id;
      userPickedAccount = true;
      render();
    });
    li.addEventListener("dblclick", () => {
      if (busy) return;
      selectedAccountId = account.id;
      userPickedAccount = true;
      void doLaunch();
    });
    els.accountList.appendChild(li);
  }

  const hasEnv = Boolean(selectedEnvId);
  const hasAcct = Boolean(selectedAccountId);
  const pending = accounts.some(
    (a) => a.pendingLogin || a.name.startsWith("Signing in"),
  );
  const alreadyRunning = isActiveSelectionRunning();

  const isCurrentEnv = Boolean(
    selectedEnvId && config.active.envId === selectedEnvId,
  );
  const onlyEnv = config.environments.length <= 1;

  els.envNew.disabled = busy;
  els.envRename.disabled = busy || !hasEnv;
  els.envDelete.disabled = busy || !hasEnv || isCurrentEnv || onlyEnv;
  els.acctNew.disabled = busy || !hasEnv;
  els.acctDelete.disabled = busy || !hasAcct;
  els.launch.disabled = busy || !hasEnv || !hasAcct || alreadyRunning;

  if (busy && busyLabel) {
    els.launch.textContent =
      busyLabel === "Closing…" ||
      busyLabel === "Launching…" ||
      busyLabel === "Copying…" ||
      busyLabel === "Creating…" ||
      busyLabel === "Deleting…"
        ? busyLabel
        : "Launch";
  } else if (alreadyRunning) {
    els.launch.textContent = "Already running";
  } else {
    els.launch.textContent = "Launch";
  }

  if (pending) {
    showHint("Waiting for sign-in… email will be captured automatically.");
  } else if (!hasAcct) {
    showHint("Add an account to sign in, or Launch after selecting one.");
  } else if (!isCurrentEnv) {
    showHint("Launch to switch Cursor’s current environment.");
  } else if (selectedAccountId !== config.active.accountId) {
    showHint("Launch to switch Cursor’s current account.");
  } else if (Date.now() > flashUntil) {
    showHint(null);
  }

  // Defer so the accounts list paints before the (slow) disk-usage IPC starts.
  window.setTimeout(() => void refreshEnvDiskUsage(false), 0);
}

async function refresh(opts?: { silent?: boolean; syncSelection?: boolean }) {
  const result = await call<ListStateResult>("list_state");
  state = result.state;
  // Only sync selection from "active" on first load — never clobber a user click.
  if (opts?.syncSelection || !userPickedAccount) {
    if (!selectedEnvId) {
      selectedEnvId = state.config.active.envId ?? state.config.environments[0]?.id ?? null;
    }
    if (!selectedAccountId || opts?.syncSelection) {
      selectedAccountId =
        state.config.active.accountId ??
        accountsForEnv(selectedEnvId)[0]?.id ??
        null;
    }
  }
  // Drop selection only if it no longer exists.
  if (selectedEnvId && !state.config.environments.some((e) => e.id === selectedEnvId)) {
    selectedEnvId = state.config.active.envId ?? state.config.environments[0]?.id ?? null;
    userPickedAccount = false;
    selectedAccountId = state.config.active.accountId ?? null;
  }
  if (
    selectedAccountId &&
    !state.config.accounts.some((a) => a.id === selectedAccountId)
  ) {
    selectedAccountId =
      (state.config.active.envId === selectedEnvId
        ? state.config.active.accountId
        : null) ??
      accountsForEnv(selectedEnvId)[0]?.id ??
      null;
  }
  if (result.capturedEmail && !opts?.silent) {
    showSuccess(`Signed in as ${result.capturedEmail}.`);
  }
  render();
}

async function withBusy(label: string, fn: () => Promise<void>) {
  if (busy) return;
  busy = true;
  busyLabel = label;
  showError(null);
  render();
  try {
    await fn();
  } finally {
    busy = false;
    busyLabel = null;
    render();
  }
}

async function confirmQuitIfNeeded(reason: string): Promise<boolean> {
  if (!state?.cursorRunning) return true;
  return askConfirm(
    "Restart Cursor?",
    `${reason}\n\nCursor will quit and reopen as needed.`,
    "Continue",
    false,
  );
}

async function waitForCursorState(
  wantRunning: boolean,
  timeoutMs = 60_000,
): Promise<boolean> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    const running = await invoke<boolean>("is_cursor_running");
    if (state) state.cursorRunning = running;
    if (running === wantRunning) return true;
    await new Promise((r) => setTimeout(r, 250));
  }
  return false;
}

async function doLaunch() {
  if (!selectedEnvId || !selectedAccountId || busy) return;
  if (isActiveSelectionRunning()) return;

  const envChanging = state?.config.active.envId !== selectedEnvId;
  const accountChanging = state?.config.active.accountId !== selectedAccountId;
  const needsRestart =
    Boolean(state?.cursorRunning) && (envChanging || accountChanging);

  if (needsRestart || (envChanging && !state?.cursorRunning)) {
    const account = state?.config.accounts.find((a) => a.id === selectedAccountId);
    const env = state?.config.environments.find((e) => e.id === selectedEnvId);
    const label = account ? accountConfirmLabel(account) : "the selected account";
    const envName = env?.name ?? "the selected environment";
    const title = envChanging ? "Switching environments" : "Switching accounts";
    const message = envChanging
      ? `Activate the “${envName}” environment with the ${label} account?`
      : `Activate the ${label} account?`;
    const ok = await askConfirm(
      title,
      state?.cursorRunning ? `${message}\n\nCursor will quit and reopen.` : message,
      state?.cursorRunning ? "Restart" : "Switch",
      false,
    );
    if (!ok) return;
  }

  if (busy) return;
  const mustQuit = needsRestart || Boolean(state?.cursorRunning);
  try {
    if (mustQuit) {
      // Dialog is already closed; show closing feedback before any quit IPC.
      await quitCursorWithUi();
    } else {
      busy = true;
      busyLabel = "Launching…";
      showError(null);
      showSuccess("Cursor is launching…", { sticky: true });
      render();
      await yieldToPaint();
    }

    busyLabel = "Launching…";
    showSuccess("Cursor is launching…", { sticky: true });
    render();
    await yieldToPaint();

    state = await call<AppState>("launch", {
      envId: selectedEnvId,
      accountId: selectedAccountId,
    });

    const opened = await waitForCursorState(true, 45_000);
    if (opened) {
      state.cursorRunning = true;
      showSuccess(null);
    } else {
      showSuccess("Launch was requested; Cursor is still starting…");
    }
  } catch (err) {
    if (typeof err === "string") showError(err);
  } finally {
    busy = false;
    busyLabel = null;
    render();
  }
}

window.addEventListener("DOMContentLoaded", () => {
  els.modalCancel.addEventListener("click", () => closeModal(null));
  els.modalConfirm.addEventListener("click", () => {
    if (els.modalInput.classList.contains("hidden")) {
      closeModal("ok");
      return;
    }
    const value = els.modalInput.value.trim();
    closeModal(value || null);
  });
  els.modalLink.addEventListener("click", (e) => {
    e.preventDefault();
    const url = els.modalLink.getAttribute("href");
    if (url && url !== "#") {
      void invoke("open_url", { url }).catch((err) => {
        showError(typeof err === "string" ? err : String(err));
      });
    }
  });
  els.modalInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      els.modalConfirm.click();
    } else if (e.key === "Escape") {
      e.preventDefault();
      closeModal(null);
    }
  });
  els.modal.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      closeModal(null);
    }
  });
  void listen("check-for-updates", () => {
    void checkForUpdates();
  });

  els.envSelect.addEventListener("change", () => {
    selectedEnvId = els.envSelect.value || null;
    selectedAccountId = null;
    userPickedAccount = false;
    diskUsageEnvId = null;
    // Drop the previous env's accounts immediately so they never linger during `du`.
    els.accountList.innerHTML = "";
    els.envDisk.textContent = "Disk usage: …";
    render();
  });

  els.envNew.addEventListener("click", async () => {
    if (busy) return;
    const result = await askTextWithCheckbox({
      title: "New environment",
      confirmLabel: "Create",
      checkboxLabel: "Copy current environment (Application Support + ~/.cursor)",
      checkboxDefault: true,
    });
    if (!result) return;

    const wantCopy = result.checked;
    const didQuitForCopy = Boolean(wantCopy && state?.cursorRunning);
    if (didQuitForCopy) {
      const ok = await askConfirm(
        "Quit Cursor?",
        "Cursor must quit before its data folders can be copied into the new environment.",
        "Quit & Create",
        false,
      );
      if (!ok) return;
    }

    let unlistenProgress: UnlistenFn | null = null;
    busy = true;
    showError(null);
    setProgress(null);
    let createdId: string | null = null;
    try {
      // 1) Create empty env and select it immediately (accounts list clears).
      busyLabel = "Creating…";
      showSuccess("Creating environment…", { sticky: true });
      render();
      await yieldToPaint();

      state = await call<AppState>("create_environment", {
        name: result.value,
      });
      const created =
        state.config.environments.find((e) => e.name === result.value) ??
        state.config.environments[state.config.environments.length - 1];
      createdId = created?.id ?? null;
      selectedEnvId = createdId;
      selectedAccountId = null;
      userPickedAccount = false;
      render();
      await yieldToPaint();

      if (wantCopy && createdId) {
        if (didQuitForCopy) {
          await quitCursorWithUi();
          await new Promise((r) => setTimeout(r, 500));
        }

        busyLabel = "Copying…";
        showSuccess(null);
        setProgress(0, "Copying…");
        unlistenProgress = await listen<CopyProgressEvent>("env-copy-progress", (ev) => {
          setProgress(ev.payload.percent, ev.payload.label);
        });
        render();
        await yieldToPaint();

        state = await call<AppState>("copy_environment_from_current", {
          envId: createdId,
        });
        // Keep selection on the new env after copy.
        selectedEnvId = createdId;
        selectedAccountId = null;
        userPickedAccount = false;
      }

      setProgress(null);
      // Do not auto-launch Cursor — new env has no accounts until the user adds one.
      diskUsageEnvId = null;
      void refreshEnvDiskUsage(true);
      showSuccess(
        wantCopy
          ? `Environment “${result.value}” created (as a copy).`
          : `Environment “${result.value}” created.`,
      );
    } catch (err) {
      setProgress(null);
      if (typeof err === "string") showError(err);
      // Keep the new env selected even if copy failed; do not auto-launch Cursor.
      if (createdId) {
        selectedEnvId = createdId;
        selectedAccountId = null;
        userPickedAccount = false;
      }
      try {
        await refresh({ silent: true });
        if (createdId) {
          selectedEnvId = createdId;
          selectedAccountId = null;
          userPickedAccount = false;
        }
      } catch {
        /* ignore */
      }
    } finally {
      if (unlistenProgress) unlistenProgress();
      busy = false;
      busyLabel = null;
      setProgress(null);
      render();
    }
  });

  els.envRename.addEventListener("click", async () => {
    if (!selectedEnvId || !state || busy) return;
    const current = state.config.environments.find((e) => e.id === selectedEnvId);
    const name = await askText("Rename environment", current?.name ?? "");
    if (!name) return;
    await withBusy("Renaming…", async () => {
      state = await call<AppState>("rename_environment", {
        id: selectedEnvId,
        name,
      });
      showSuccess(`Renamed to “${name}”.`);
    });
  });

  els.envDelete.addEventListener("click", async () => {
    if (!selectedEnvId || !state || busy) return;
    if (selectedEnvId === state.config.active.envId) {
      showError("Cannot delete the current environment. Switch to another one first.");
      return;
    }
    const current = state.config.environments.find((e) => e.id === selectedEnvId);
    const ok = await askConfirm(
      "Delete environment?",
      `Move “${current?.name ?? "this environment"}” (inactive data + ~/.cursor pool + saved logins) to Trash?`,
      "Move to Trash",
      true,
    );
    if (!ok) return;

    let unlistenProgress: UnlistenFn | null = null;
    busy = true;
    busyLabel = "Deleting…";
    showError(null);
    showSuccess(null);
    setProgress(0, "Deleting…");
    render();
    await yieldToPaint();
    try {
      unlistenProgress = await listen<CopyProgressEvent>("env-copy-progress", (ev) => {
        setProgress(ev.payload.percent, ev.payload.label);
      });
      state = await call<AppState>("delete_environment", { id: selectedEnvId });
      selectedEnvId = state.config.active.envId ?? null;
      selectedAccountId = state.config.active.accountId ?? null;
      userPickedAccount = false;
      diskUsageEnvId = null;
      setProgress(null);
      showSuccess("Environment moved to Trash.");
    } catch (err) {
      setProgress(null);
      if (typeof err === "string") showError(err);
    } finally {
      if (unlistenProgress) unlistenProgress();
      busy = false;
      busyLabel = null;
      setProgress(null);
      render();
    }
  });

  els.acctNew.addEventListener("click", async () => {
    if (!selectedEnvId || busy) return;
    if (state?.cursorRunning) {
      const ok = await askConfirm(
        "Add account?",
        "Cursor will quit and reopen so you can sign in. The account name will be set from the email after login.",
        "Restart",
        false,
      );
      if (!ok) return;
      try {
        await quitCursorWithUi();
      } catch (err) {
        if (typeof err === "string") showError(err);
        busy = false;
        busyLabel = null;
        render();
        return;
      }
    } else {
      const ok = await askConfirm(
        "Add account?",
        "Cursor will open so you can sign in. The account name will be set from the email after login.",
        "Create & Launch",
        false,
      );
      if (!ok) return;
    }
    await withBusy("Launching…", async () => {
      els.launch.textContent = "Launching…";
      state = await call<AppState>("create_account", {
        envId: selectedEnvId,
      });
      selectedAccountId = state.config.active.accountId ?? null;
      userPickedAccount = true;
      showSuccess("Cursor opened for sign-in. Waiting for email…");
    });
  });

  els.acctDelete.addEventListener("click", async () => {
    if (!selectedAccountId || !state || busy) return;
    const current = state.config.accounts.find((a) => a.id === selectedAccountId);
    if (!current) return;
    const isActive = state.config.active.accountId === current.id;
    const ok = await askConfirm(
      "Delete account?",
      isActive
        ? `Delete “${accountDisplayName(current)}”? This is the current account — Cursor will quit so its login can be cleared.`
        : `Remove “${accountDisplayName(current)}” from Multi Cursor? Only the saved login snapshot is deleted; Cursor is unaffected.`,
      "Delete",
      true,
    );
    if (!ok) return;
    if (isActive && state.cursorRunning) {
      if (
        !(await confirmQuitIfNeeded(
          "Deleting the current account requires Cursor to quit first.",
        ))
      ) {
        return;
      }
      try {
        await quitCursorWithUi();
      } catch (err) {
        if (typeof err === "string") showError(err);
        busy = false;
        busyLabel = null;
        render();
        return;
      }
      busy = false;
      busyLabel = null;
    }
    await withBusy("Deleting…", async () => {
      state = await call<AppState>("delete_account", { id: selectedAccountId });
      selectedAccountId = state.config.active.accountId ?? null;
      userPickedAccount = false;
      showSuccess("Account deleted.");
    });
  });

  els.launch.addEventListener("click", () => void doLaunch());

  void refresh({ syncSelection: true }).catch(() => undefined);

  window.setInterval(() => {
    if (busy) return;
    // Poll while waiting for sign-in, or while an account is still labeled
    // "Signing in…" (covers the stuck case where tokens arrived before email).
    const waitingSignIn = accountsForEnv(selectedEnvId).some(
      (a) => a.pendingLogin || a.name.startsWith("Signing in"),
    );
    if (waitingSignIn) {
      void refresh({ silent: true })
        .then(() => {
          const stillWaiting = accountsForEnv(selectedEnvId).some(
            (a) => a.pendingLogin || a.name.startsWith("Signing in"),
          );
          if (!stillWaiting) {
            const email =
              state?.config.accounts.find(
                (a) => a.id === state?.config.active.accountId,
              )?.email ??
              accountsForEnv(selectedEnvId).find((a) => a.email)?.email ??
              null;
            if (email) showSuccess(`Signed in as ${email}.`);
            render();
          }
        })
        .catch(() => undefined);
      return;
    }
    void invoke<boolean>("is_cursor_running")
      .then((running) => {
        if (!state) return;
        if (state.cursorRunning !== running) {
          state.cursorRunning = running;
          render();
        }
      })
      .catch(() => undefined);
  }, 2000);
});
