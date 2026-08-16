window.__ModuleLoader__.load({
  id: "@dsh-desktop/theme-settings",
  factory: (require) => {
    const module = { exports: {} };
    const exports = module.exports;
    Object.defineProperty(exports, Symbol.toStringTag, { value: "Module" });
    const React = require("react");
    const { jsx, jsxs } = require("react/jsx-runtime");
    const NS = "desktop-settings";
    const inject = ["slots", "locale"];
    const API_PREFIX = "/api/desktop-managed-plugins";
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
`;
      document.head.appendChild(style);
    }

    async function request(path, options) {
      const response = await fetch(`${API_PREFIX}/${path}`, options);
      const body = await response.json();
      if (!response.ok || body.ok !== true) throw new Error(body.error || "request-failed");
      return body;
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

    /** 渲染上游 Skin Center 使用的子槽位，不复制其界面和 Host API。 */
    function ThemeSettingsSection({ renderSlot }) {
      return jsx("section", {
        id: "desktop-theme",
        "data-dsh-desktop-theme-settings": "",
        children: renderSlot("web-ui.plugin.item", {}),
      });
    }

    /** 注册两个一级设置入口，并声明 Skin Center 0.1.17 所需的稳定子槽位。 */
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
