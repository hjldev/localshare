function escapeHtml(value) {
  return String(value ?? "").replace(/[&<>"']/g, (ch) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;",
  }[ch]));
}

function normalizePath(path) {
  if (!path || path === "/") return "/";
  return path.startsWith("/") ? path : `/${path}`;
}

function splitPath(path) {
  return normalizePath(path).split("/").filter(Boolean);
}

function buttonEl(className, label, onClick) {
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = className;
  btn.textContent = label;
  btn.addEventListener("click", onClick);
  return btn;
}

function actionEl(action) {
  if (action.href) {
    const link = document.createElement("a");
    link.className = `lsb-action-btn ${action.kind || "ghost"}`;
    link.textContent = action.label;
    link.href = action.href;
    if (action.targetBlank) {
      link.target = "_blank";
      link.rel = "noreferrer";
    }
    return link;
  }

  return buttonEl(`lsb-action-btn ${action.kind || "ghost"}`, action.label, action.onClick);
}

export function createSharedFileBrowser(options) {
  const {
    mount,
    onNavigate,
    getTopActions,
    getItemActions,
    emptyText = "这个目录是空的。",
  } = options;

  const state = {
    path: "/",
    entries: [],
    statusText: "",
    loadingText: "",
    errorText: "",
  };

  mount.innerHTML = `
    <section class="lsb-root">
      <div class="lsb-toolbar">
        <div class="lsb-crumbs"></div>
        <div class="lsb-top-actions"></div>
      </div>
      <div class="lsb-content">
        <div class="lsb-status"></div>
        <div class="lsb-body"></div>
      </div>
    </section>
  `;

  const crumbsEl = mount.querySelector(".lsb-crumbs");
  const topActionsEl = mount.querySelector(".lsb-top-actions");
  const statusEl = mount.querySelector(".lsb-status");
  const bodyEl = mount.querySelector(".lsb-body");

  function renderBreadcrumbs() {
    crumbsEl.innerHTML = "";

    crumbsEl.appendChild(buttonEl("lsb-crumb-btn", "根目录", () => onNavigate?.("/")));
    let built = "";
    for (const seg of splitPath(state.path)) {
      const sep = document.createElement("span");
      sep.className = "lsb-sep";
      sep.textContent = "/";
      crumbsEl.appendChild(sep);

      built += `/${seg}`;
      crumbsEl.appendChild(buttonEl("lsb-crumb-btn", seg, () => onNavigate?.(built)));
    }
  }

  function renderTopActions() {
    topActionsEl.innerHTML = "";
    for (const action of getTopActions?.(state.path) || []) {
      topActionsEl.appendChild(buttonEl(`lsb-top-btn ${action.kind || ""}`.trim(), action.label, action.onClick));
    }
  }

  function renderBody() {
    statusEl.textContent = state.statusText;

    if (state.loadingText) {
      bodyEl.innerHTML = `<div class="lsb-loading">${escapeHtml(state.loadingText)}</div>`;
      return;
    }

    if (state.errorText) {
      bodyEl.innerHTML = `<div class="lsb-error">${escapeHtml(state.errorText)}</div>`;
      return;
    }

    if (!state.entries.length) {
      bodyEl.innerHTML = `<div class="lsb-empty">${escapeHtml(emptyText)}</div>`;
      return;
    }

    const listEl = document.createElement("div");
    listEl.className = "lsb-list";

    for (const entry of state.entries) {
      const card = document.createElement("article");
      card.className = "lsb-card";

      const main = document.createElement("div");
      main.className = "lsb-main";
      main.innerHTML = `
        <div class="lsb-icon">${escapeHtml(entry.icon || "📄")}</div>
        <div class="lsb-meta">
          <div class="lsb-name">${escapeHtml(entry.name)}</div>
          <div class="lsb-detail">${escapeHtml(entry.is_dir ? `文件夹 · 最后修改 ${entry.modified}` : `${entry.size_human} · 最后修改 ${entry.modified}`)}</div>
        </div>
      `;
      card.appendChild(main);

      const actionsWrap = document.createElement("div");
      actionsWrap.className = "lsb-item-actions";
      const itemActions = getItemActions?.(entry) ?? [];

      if (entry.is_dir && itemActions.length === 0) {
        itemActions.push({
          label: "进入目录",
          kind: "secondary",
          onClick: () => onNavigate?.(entry.path),
        });
      }

      for (const action of itemActions) {
        actionsWrap.appendChild(actionEl(action));
      }
      card.appendChild(actionsWrap);
      listEl.appendChild(card);
    }

    bodyEl.innerHTML = "";
    bodyEl.appendChild(listEl);
  }

  function redraw() {
    renderBreadcrumbs();
    renderTopActions();
    renderBody();
  }

  redraw();

  return {
    setLoading(text = "正在加载...") {
      state.loadingText = text;
      state.errorText = "";
      redraw();
    },
    setError(text) {
      state.loadingText = "";
      state.errorText = text;
      redraw();
    },
    setData({ path, entries, statusText }) {
      state.path = normalizePath(path);
      state.entries = Array.isArray(entries) ? entries : [];
      state.statusText = statusText || `${state.entries.length} 个项目`;
      state.loadingText = "";
      state.errorText = "";
      redraw();
    },
    refresh() {
      redraw();
    },
  };
}
