# Third-Party Notices

DeepSeek Harness Desktop is a community project and is not an official DeepSeek product.

The preview installer bundles the following third-party software:

| Component | Pinned version | License / source |
|---|---:|---|
| Node.js | 22.22.3 | Node.js license, <https://github.com/nodejs/node> |
| `@deepseek-ai/dsh` | 0.1.0-rc.6 | MIT, <https://www.npmjs.com/package/@deepseek-ai/dsh> |
| `dshmarket` | 1.6.0 | MIT, <https://github.com/dsh-market/dsh-market> |
| `pnpm` | 11.22.0 | MIT, <https://github.com/pnpm/pnpm> |
| `dsh-at-file` | 0.6.0 | MIT, <https://github.com/omdsh-dev/dsh-at-file> |
| `@omdsh-dev/dsh-genui` | 0.8.4 | MIT, <https://github.com/omdsh-dev/dsh-genui> |
| `dsh-better-sidebar` | 0.12.2 | MIT, <https://github.com/omdsh-dev/DSH-better-sidebar> |
| `@linxin666/dsh-skins` and Skin Center | 0.1.17 | Apache-2.0, <https://github.com/zhu1090093659/dsh-web-ui> |
| `@vectorize-io/hindsight-coding-agents` | 0.3.4 | MIT, <https://github.com/vectorize-io/hindsight/tree/main/hindsight-integrations/coding-agents> |
| `@liustack/modlens` | 3.16.7 | MIT, <https://github.com/liustack/modlens> |
| `@zebbkira/dsh-skills-mcp-manager` | 0.1.3 | MIT, <https://github.com/zebbkira/dsh-skills-mcp-manager> |
| Tauri | 2.x | Apache-2.0 OR MIT, <https://github.com/tauri-apps/tauri> |

The build pipeline copies the licenses shipped with the pinned Node.js archive and npm dependency tree into the packaged runtime. Those upstream license texts govern their respective components.

The managed plugins can access capabilities exposed by the DSH Host. In particular, Better Sidebar can read and write workspace files, invoke Git, and create local PTY processes; Skin Center can update `$DSH_HOME/cordis.patch.yml`; GenUI actions send user interaction data back to the active model; Hindsight can send opted-in project memory to the endpoint configured by the user; ModLens can send user-selected images to a configured vision service; Skills/MCP Manager can delete Skills and stores MCP `env` and `headers` in plaintext at `~/.dsh/mcp.json`. DSH Desktop disables install scripts for all managed npm plugins, defaults Hindsight to an empty opt-in list, and initializes Better Sidebar HTTP/HTTPS interception to off for a first managed installation. Plugins installed through DSH Market run with the same host permissions as the desktop application and are not protected by package signatures, permission manifests, or a process sandbox.
