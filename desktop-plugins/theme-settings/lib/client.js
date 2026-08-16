window.__ModuleLoader__.load({
  id: "@dsh-desktop/theme-settings",
  factory: (require) => {
    const module = { exports: {} };
    const exports = module.exports;
    Object.defineProperty(exports, Symbol.toStringTag, { value: "Module" });
    const React = require("react");
    const { jsx, jsxs } = require("react/jsx-runtime");
    const NS = "desktop-settings";
    const inject = ["slots", "workspaces", "locale"];
    const API_PREFIX = "/api/desktop-managed-plugins";
    const MEMORY_API_PREFIX = "/api/desktop-hindsight";
    const STYLE_ID = "@dsh-desktop/theme-settings/client.css";

    if (document.querySelector(`style[data-plugin-css="${STYLE_ID}"]`) === null) {
      const style = document.createElement("style");
      style.dataset.pluginCss = STYLE_ID;
      style.textContent = `
.dsh_desktop_plugins{display:flex;flex-direction:column;gap:0;max-width:860px}
.dsh_desktop_plugins h2{font-size:20px;line-height:1.4;margin:0 0 8px}
.dsh_desktop_plugins_intro,.dsh_desktop_plugins_state{color:var(--dsw-alias-label-secondary);font-size:14px;line-height:1.6;margin:0 0 18px}
.dsh_desktop_plugin_row{display:flex;align-items:center;justify-content:space-between;gap:24px;min-height:66px;border-top:1px solid var(--dsw-alias-border-l1,#e5e7eb)}
.dsh_desktop_plugin_row:last-of-type{border-bottom:1px solid var(--dsw-alias-border-l1,#e5e7eb)}
.dsh_desktop_plugin_copy{display:flex;flex-direction:column;gap:3px;min-width:0}
.dsh_desktop_plugin_name{font-size:15px;color:var(--dsw-alias-label-primary);font-weight:600}
.dsh_desktop_plugin_package{font-size:12px;color:var(--dsw-alias-label-tertiary);overflow-wrap:anywhere}
.dsh_desktop_switch{position:relative;width:42px;height:24px;border:0;border-radius:12px;padding:0;background:var(--dsw-alias-bg-layer-2,#d1d5db);cursor:pointer;flex:0 0 auto}
.dsh_desktop_switch[aria-checked="true"]{background:var(--dsw-alias-state-business-primary,#4f6ef7)}
.dsh_desktop_switch:disabled{cursor:not-allowed;opacity:.5}
.dsh_desktop_switch span{position:absolute;width:18px;height:18px;left:3px;top:3px;border-radius:50%;background:#fff;transition:transform .16s}
.dsh_desktop_switch[aria-checked="true"] span{transform:translateX(18px)}
.dsh_desktop_notice{margin-top:16px;color:var(--dsw-alias-state-warn-primary,#9a6700);font-size:13px}
.dsh_desktop_retry{border:1px solid var(--dsw-alias-border-l2,#d1d5db);border-radius:6px;background:transparent;color:inherit;padding:6px 12px;cursor:pointer}
.dsh_desktop_memory{display:flex;flex-direction:column;gap:22px;max-width:860px;padding-bottom:24px}
.dsh_desktop_memory h2{font-size:20px;line-height:1.4;margin:0}
.dsh_desktop_memory_intro{color:var(--dsw-alias-label-secondary);font-size:14px;line-height:1.6;margin:-14px 0 0}
.dsh_desktop_memory_group{display:flex;flex-direction:column;gap:12px;padding-top:18px;border-top:1px solid var(--dsw-alias-border-l1,#e5e7eb)}
.dsh_desktop_memory_group h3{font-size:15px;line-height:1.4;margin:0;font-weight:650}
.dsh_desktop_segmented{display:grid;grid-template-columns:1fr 1fr;gap:4px;width:min(360px,100%);padding:4px;background:var(--dsw-alias-bg-layer-1,#f3f4f6);border-radius:6px}
.dsh_desktop_segmented button{height:36px;border:0;border-radius:4px;background:transparent;color:var(--dsw-alias-label-secondary);cursor:pointer;font:inherit}
.dsh_desktop_segmented button[aria-pressed="true"]{background:var(--dsw-alias-bg-layer-3,#fff);color:var(--dsw-alias-label-primary);box-shadow:0 1px 2px rgba(0,0,0,.08);font-weight:600}
.dsh_desktop_field{display:flex;flex-direction:column;gap:7px}
.dsh_desktop_field label{font-size:13px;color:var(--dsw-alias-label-secondary)}
.dsh_desktop_input{box-sizing:border-box;width:100%;height:40px;border:1px solid var(--dsw-alias-border-l2,#d1d5db);border-radius:6px;background:var(--dsw-alias-bg-layer-1,#fff);color:var(--dsw-alias-label-primary);padding:0 12px;font:inherit;outline:none}
.dsh_desktop_input:focus{border-color:var(--dsw-alias-state-business-primary,#4f6ef7);box-shadow:0 0 0 2px color-mix(in srgb,var(--dsw-alias-state-business-primary,#4f6ef7) 18%,transparent)}
.dsh_desktop_credential_line{display:grid;grid-template-columns:minmax(0,1fr) auto;gap:10px;align-items:end}
.dsh_desktop_status{font-size:13px;color:var(--dsw-alias-label-secondary);margin:0}
.dsh_desktop_status[data-kind="success"]{color:var(--dsw-alias-state-success-primary,#17803d)}
.dsh_desktop_status[data-kind="error"]{color:var(--dsw-alias-state-danger-primary,#c9362b)}
.dsh_desktop_actions{display:flex;align-items:center;gap:10px;flex-wrap:wrap}
.dsh_desktop_button{display:inline-flex;align-items:center;justify-content:center;min-height:38px;border:1px solid var(--dsw-alias-border-l2,#d1d5db);border-radius:6px;padding:0 15px;background:var(--dsw-alias-bg-layer-1,#fff);color:var(--dsw-alias-label-primary);font:inherit;font-weight:550;cursor:pointer}
.dsh_desktop_button[data-variant="primary"]{border-color:var(--dsw-alias-state-business-primary,#4f6ef7);background:var(--dsw-alias-state-business-primary,#4f6ef7);color:#fff}
.dsh_desktop_button:disabled{opacity:.5;cursor:not-allowed}
.dsh_desktop_projects{display:flex;flex-direction:column;border-top:1px solid var(--dsw-alias-border-l1,#e5e7eb)}
.dsh_desktop_project{display:grid;grid-template-columns:minmax(0,1fr) auto;gap:18px;align-items:center;min-height:60px;border-bottom:1px solid var(--dsw-alias-border-l1,#e5e7eb)}
.dsh_desktop_project_copy{display:flex;flex-direction:column;min-width:0;gap:2px}
.dsh_desktop_project_name{font-size:14px;font-weight:600;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.dsh_desktop_project_path{font-size:12px;color:var(--dsw-alias-label-tertiary);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.dsh_desktop_empty{padding:22px 0;color:var(--dsw-alias-label-secondary);font-size:14px}
@media(max-width:720px){.dsh_desktop_credential_line{grid-template-columns:1fr}.dsh_desktop_project{gap:10px}.dsh_desktop_project_path{white-space:normal;overflow-wrap:anywhere}}
`;
      document.head.appendChild(style);
    }

    async function request(path, options, prefix = API_PREFIX) {
      const response = await fetch(`${prefix}/${path}`, options);
      const body = await response.json();
      if (!response.ok || body.ok !== true) throw new Error(body.error || "request-failed");
      return body;
    }

    async function memoryRequest(path, options) {
      return request(path, options, MEMORY_API_PREFIX);
    }

    /** 展示精确白名单中的桌面托管 bundle，开关仅修改 profile bundles。 */
    function ManagedPluginsSection({ t }) {
      const [plugins, setPlugins] = React.useState([]);
      const [error, setError] = React.useState("");
      const [pending, setPending] = React.useState("");
      const [restartRequired, setRestartRequired] = React.useState(false);
      const load = React.useCallback(() => {
        setError("");
        request("state").then(
          (body) => setPlugins(body.plugins),
          (failure) => setError(failure.message),
        );
      }, []);
      React.useEffect(load, [load]);

      const toggle = async (plugin) => {
        setPending(plugin.package);
        setError("");
        try {
          const body = await request("toggle", {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({
              profile: "web",
              package: plugin.package,
              enabled: !plugin.enabled,
            }),
          });
          setPlugins((current) =>
            current.map((item) =>
              item.package === plugin.package ? { ...item, enabled: body.enabled } : item,
            ),
          );
          setRestartRequired(true);
        } catch (failure) {
          setError(failure.message);
        } finally {
          setPending("");
        }
      };

      return jsxs("section", {
        id: "desktop-managed-plugins",
        className: "dsh_desktop_plugins",
        "aria-labelledby": "desktop-managed-plugins-title",
        children: [
          jsx("h2", { id: "desktop-managed-plugins-title", children: t("plugins.title") }),
          jsx("p", { className: "dsh_desktop_plugins_intro", children: t("plugins.intro") }),
          error !== ""
            ? jsxs("div", {
                className: "dsh_desktop_plugins_state",
                role: "alert",
                children: [
                  jsx("span", { children: t("plugins.error", { error }) }),
                  " ",
                  jsx("button", {
                    type: "button",
                    className: "dsh_desktop_retry",
                    onClick: load,
                    children: t("plugins.retry"),
                  }),
                ],
              })
            : null,
          error === "" && plugins.length === 0
            ? jsx("p", { className: "dsh_desktop_plugins_state", children: t("plugins.loading") })
            : null,
          ...plugins.map((plugin) =>
            jsxs("div", {
              className: "dsh_desktop_plugin_row",
              children: [
                jsxs("div", {
                  className: "dsh_desktop_plugin_copy",
                  children: [
                    jsx("span", { className: "dsh_desktop_plugin_name", children: plugin.label }),
                    jsx("code", { className: "dsh_desktop_plugin_package", children: plugin.package }),
                  ],
                }),
                jsx("button", {
                  type: "button",
                  role: "switch",
                  className: "dsh_desktop_switch",
                  "aria-checked": plugin.enabled,
                  "aria-label": t(plugin.enabled ? "plugins.disable" : "plugins.enable", { name: plugin.label }),
                  title: t(plugin.enabled ? "plugins.disable" : "plugins.enable", { name: plugin.label }),
                  disabled: pending !== "",
                  onClick: () => void toggle(plugin),
                  children: jsx("span", { "aria-hidden": true }),
                }),
              ],
            }, plugin.package),
          ),
          restartRequired
            ? jsx("p", {
                className: "dsh_desktop_notice",
                role: "status",
                children: t("plugins.restart"),
              })
            : null,
        ],
      });
    }

    /** 通过桌面凭据库和 DSH 工作区服务配置项目级 Hindsight 显式启用。 */
    function MemorySettingsSection({ t, workspaceList, pickDirectory }) {
      const snapshot = React.useSyncExternalStore(
        React.useCallback((listener) => workspaceList.subscribe(listener), [workspaceList]),
        React.useCallback(() => workspaceList.getSnapshot(), [workspaceList]),
      );
      const [mode, setMode] = React.useState("cloud");
      const [apiUrl, setApiUrl] = React.useState("https://api.hindsight.vectorize.io");
      const [optInPaths, setOptInPaths] = React.useState([]);
      const [credential, setCredential] = React.useState({ configured: false, writable: true });
      const [token, setToken] = React.useState("");
      const [pending, setPending] = React.useState("");
      const [status, setStatus] = React.useState({ kind: "", text: "" });

      const load = React.useCallback(async () => {
        setPending("load");
        setStatus({ kind: "", text: "" });
        try {
          const body = await memoryRequest("state");
          setMode(body.config.serverMode);
          setApiUrl(body.config.apiUrl);
          setOptInPaths(body.config.optInPaths);
          setCredential(body.credential);
        } catch (error) {
          setStatus({ kind: "error", text: t("memory.loadError", { error: error.message }) });
        } finally {
          setPending("");
        }
      }, [t]);
      React.useEffect(() => { void load(); }, [load]);

      const submitConfig = async () => {
        setPending("save");
        setStatus({ kind: "", text: "" });
        try {
          await memoryRequest("config", {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ serverMode: mode, apiUrl, optInPaths }),
          });
          if (token.trim() !== "") {
            const result = await memoryRequest("credential", {
              method: "POST",
              headers: { "content-type": "application/json" },
              body: JSON.stringify({ token }),
            });
            setCredential(result.credential);
            setToken("");
          }
          setStatus({ kind: "success", text: t("memory.saved") });
        } catch (error) {
          setStatus({ kind: "error", text: t("memory.saveError", { error: error.message }) });
        } finally {
          setPending("");
        }
      };

      const testConnection = async () => {
        setPending("test");
        setStatus({ kind: "", text: "" });
        try {
          const body = await memoryRequest("test", {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ serverMode: mode, apiUrl, optInPaths }),
          });
          setStatus({ kind: "success", text: t("memory.testOk", { version: body.version }) });
        } catch (error) {
          setStatus({ kind: "error", text: t("memory.testError", { error: error.message }) });
        } finally {
          setPending("");
        }
      };

      const addDirectory = async () => {
        setPending("pick");
        try {
          const path = await pickDirectory();
          if (path !== null) setOptInPaths((current) => current.includes(path) ? current : [...current, path]);
        } catch (error) {
          setStatus({ kind: "error", text: t("memory.pickError", { error: error.message }) });
        } finally {
          setPending("");
        }
      };

      const workspaces = Array.isArray(snapshot?.items) ? snapshot.items : [];
      const candidates = workspaces.filter((workspace) => !optInPaths.includes(workspace.path));
      const rows = optInPaths.map((path) => {
        const workspace = workspaces.find((item) => item.path === path);
        return { path, title: workspace?.title || path.split(/[\\/]/).filter(Boolean).at(-1) || path };
      });
      const addWorkspace = (path) => setOptInPaths((current) => current.includes(path) ? current : [...current, path]);
      const removeWorkspace = (path) => setOptInPaths((current) => current.filter((item) => item !== path));
      const busy = pending !== "";

      return jsxs("section", {
        id: "desktop-memory",
        className: "dsh_desktop_memory",
        "aria-labelledby": "desktop-memory-title",
        children: [
          jsx("h2", { id: "desktop-memory-title", children: t("memory.title") }),
          jsx("p", { className: "dsh_desktop_memory_intro", children: t("memory.intro") }),
          jsxs("div", { className: "dsh_desktop_memory_group", children: [
            jsx("h3", { children: t("memory.service") }),
            jsxs("div", { className: "dsh_desktop_segmented", role: "group", "aria-label": t("memory.service"), children: [
              jsx("button", { type: "button", "aria-pressed": mode === "cloud", disabled: busy, onClick: () => { setMode("cloud"); setApiUrl("https://api.hindsight.vectorize.io"); }, children: t("memory.cloud") }),
              jsx("button", { type: "button", "aria-pressed": mode === "self-hosted", disabled: busy, onClick: () => setMode("self-hosted"), children: t("memory.selfHosted") }),
            ] }),
            mode === "self-hosted" ? jsxs("div", { className: "dsh_desktop_field", children: [
              jsx("label", { htmlFor: "desktop-memory-url", children: t("memory.url") }),
              jsx("input", { id: "desktop-memory-url", className: "dsh_desktop_input", type: "url", value: apiUrl, disabled: busy, onChange: (event) => setApiUrl(event.target.value), placeholder: "http://127.0.0.1:8888" }),
            ] }) : null,
            jsxs("div", { className: "dsh_desktop_credential_line", children: [
              jsxs("div", { className: "dsh_desktop_field", children: [
                jsx("label", { htmlFor: "desktop-memory-token", children: t("memory.token", { state: credential.configured ? t("memory.configured") : t("memory.notConfigured") }) }),
                jsx("input", { id: "desktop-memory-token", className: "dsh_desktop_input", type: "password", autoComplete: "new-password", value: token, disabled: busy || !credential.writable, onChange: (event) => setToken(event.target.value), placeholder: credential.configured ? "********" : "" }),
              ] }),
              jsx("button", { type: "button", className: "dsh_desktop_button", disabled: busy, onClick: () => void testConnection(), children: pending === "test" ? t("memory.testing") : t("memory.test") }),
            ] }),
          ] }),
          jsxs("div", { className: "dsh_desktop_memory_group", children: [
            jsx("h3", { children: t("memory.projects") }),
            jsxs("div", { className: "dsh_desktop_actions", children: [
              candidates.length > 0 ? jsx("select", { className: "dsh_desktop_input", style: { width: "auto", minWidth: "220px" }, defaultValue: "", disabled: busy, "aria-label": t("memory.addWorkspace"), onChange: (event) => { if (event.target.value !== "") { addWorkspace(event.target.value); event.target.value = ""; } }, children: [jsx("option", { value: "", children: t("memory.addWorkspace") }), ...candidates.map((workspace) => jsx("option", { value: workspace.path, children: workspace.title }, workspace.workspaceId))] }) : null,
              jsx("button", { type: "button", className: "dsh_desktop_button", disabled: busy, onClick: () => void addDirectory(), children: t("memory.chooseDirectory") }),
            ] }),
            rows.length === 0 ? jsx("div", { className: "dsh_desktop_empty", children: t("memory.empty") }) : jsx("div", { className: "dsh_desktop_projects", children: rows.map((row) => jsxs("div", { className: "dsh_desktop_project", children: [
              jsxs("div", { className: "dsh_desktop_project_copy", children: [jsx("span", { className: "dsh_desktop_project_name", children: row.title }), jsx("span", { className: "dsh_desktop_project_path", children: row.path })] }),
              jsx("button", { type: "button", className: "dsh_desktop_button", disabled: busy, onClick: () => removeWorkspace(row.path), children: t("memory.remove") }),
            ] }, row.path)) }),
          ] }),
          jsxs("div", { className: "dsh_desktop_actions", children: [
            jsx("button", { type: "button", className: "dsh_desktop_button", "data-variant": "primary", disabled: busy, onClick: () => void submitConfig(), children: pending === "save" ? t("memory.saving") : t("memory.save") }),
            status.text !== "" ? jsx("p", { className: "dsh_desktop_status", "data-kind": status.kind, role: status.kind === "error" ? "alert" : "status", children: status.text }) : null,
          ] }),
        ],
      });
    }

    /** 渲染上游 Skin Center 使用的子槽位，不复制其界面和 Host API。 */
    function ThemeSettingsSection({ renderSlot }) {
      return jsx("section", {
        id: "desktop-theme",
        "data-dsh-desktop-theme-settings": "",
        children: renderSlot("web-ui.plugin.item", {}),
      });
    }

    /** 注册桌面插件、项目记忆和主题入口，并声明 Skin Center 所需子槽位。 */
    function apply(ctx) {
      ctx.effect(
        () =>
          ctx.locale.register(NS, {
            zh: {
              "plugins.nav": "预置插件",
              "plugins.title": "预置插件",
              "plugins.intro": "开关只改变插件是否随 DSH 启动，不会卸载文件或修改依赖。",
              "plugins.loading": "正在读取插件状态...",
              "plugins.error": "读取失败：{error}",
              "plugins.retry": "重试",
              "plugins.enable": "启用 {name}",
              "plugins.disable": "停用 {name}",
              "plugins.restart": "设置已保存。请从托盘选择“重启 DSH 服务”后生效。",
              "memory.nav": "项目记忆",
              "memory.title": "项目记忆",
              "memory.intro": "仅对明确启用的项目保留和召回上下文。",
              "memory.service": "Hindsight 服务",
              "memory.cloud": "Cloud",
              "memory.selfHosted": "自托管",
              "memory.url": "服务地址",
              "memory.token": "API Token（{state}）",
              "memory.configured": "已配置",
              "memory.notConfigured": "未配置",
              "memory.test": "测试连接",
              "memory.testing": "测试中...",
              "memory.projects": "启用项目",
              "memory.addWorkspace": "从工作区添加",
              "memory.chooseDirectory": "选择目录",
              "memory.remove": "移除",
              "memory.empty": "尚未启用任何项目。",
              "memory.save": "保存",
              "memory.saving": "保存中...",
              "memory.saved": "已保存，重启 DSH 服务后生效。",
              "memory.testOk": "连接成功（{version}）",
              "memory.loadError": "读取失败：{error}",
              "memory.saveError": "保存失败：{error}",
              "memory.testError": "连接失败：{error}",
              "memory.pickError": "目录选择失败：{error}",
              "theme.nav": "主题",
            },
            en: {
              "plugins.nav": "Bundled plugins",
              "plugins.title": "Bundled plugins",
              "plugins.intro": "Switches only change startup activation; files and dependencies stay installed.",
              "plugins.loading": "Loading plugin status...",
              "plugins.error": "Could not load status: {error}",
              "plugins.retry": "Retry",
              "plugins.enable": "Enable {name}",
              "plugins.disable": "Disable {name}",
              "plugins.restart": "Saved. Restart the DSH service from the tray to apply.",
              "memory.nav": "Project memory",
              "memory.title": "Project memory",
              "memory.intro": "Retain and recall context only for explicitly enabled projects.",
              "memory.service": "Hindsight service",
              "memory.cloud": "Cloud",
              "memory.selfHosted": "Self-hosted",
              "memory.url": "Service URL",
              "memory.token": "API token ({state})",
              "memory.configured": "configured",
              "memory.notConfigured": "not configured",
              "memory.test": "Test connection",
              "memory.testing": "Testing...",
              "memory.projects": "Enabled projects",
              "memory.addWorkspace": "Add from workspaces",
              "memory.chooseDirectory": "Choose directory",
              "memory.remove": "Remove",
              "memory.empty": "No project is enabled yet.",
              "memory.save": "Save",
              "memory.saving": "Saving...",
              "memory.saved": "Saved. Restart the DSH service to apply.",
              "memory.testOk": "Connected ({version})",
              "memory.loadError": "Could not load: {error}",
              "memory.saveError": "Could not save: {error}",
              "memory.testError": "Connection failed: {error}",
              "memory.pickError": "Directory picker failed: {error}",
              "theme.nav": "Themes",
            },
          }),
        "desktop-settings: dictionaries",
      );
      const t = ctx.locale.bind(NS);
      ctx.slots.inject("settings.section", function* () {
        yield ctx.slots.register(
          {
            name: "settings.section",
            id: "desktop-memory",
            order: 17,
            label: () => t("memory.nav"),
            locale: NS,
            inject: () => ({
              workspaceList: ctx.workspaces.list,
              pickDirectory: () => ctx.workspaces.pickDirectory(),
            }),
          },
          MemorySettingsSection,
        );
        yield ctx.slots.register(
          {
            name: "settings.section",
            id: "desktop-managed-plugins",
            order: 16,
            label: () => t("plugins.nav"),
            locale: NS,
          },
          ManagedPluginsSection,
        );
        yield ctx.slots.register(
          {
            name: "settings.section",
            id: "desktop-theme",
            order: 18,
            label: () => t("theme.nav"),
            locale: NS,
            children: {
              "web-ui.plugin.item": {
                kind: "list",
                scope: "root",
              },
            },
          },
          ThemeSettingsSection,
        );
      });
    }

    exports.apply = apply;
    exports.inject = inject;
    return module.exports;
  },
});
