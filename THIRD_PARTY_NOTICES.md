# Third-Party Notices

DeepSeek Harness Desktop is a community project and is not an official DeepSeek product.

The preview installer bundles the following third-party software:

| Component | Pinned version | License / source |
|---|---:|---|
| Node.js | 22.22.3 | Node.js license, <https://github.com/nodejs/node> |
| `@deepseek-ai/dsh` | 0.1.0-rc.6 | MIT, <https://www.npmjs.com/package/@deepseek-ai/dsh> |
| `dsh-at-file` | 0.6.0 | MIT, <https://github.com/omdsh-dev/dsh-at-file> |
| `@omdsh-dev/dsh-genui` | 0.8.4 | MIT, <https://github.com/omdsh-dev/dsh-genui> |
| `dsh-better-sidebar` | 0.12.2 | MIT, <https://github.com/omdsh-dev/DSH-better-sidebar> |
| `@linxin666/dsh-skins` and Skin Center | 0.1.16 | Apache-2.0, <https://github.com/zhu1090093659/dsh-web-ui> |
| Tauri | 2.x | Apache-2.0 OR MIT, <https://github.com/tauri-apps/tauri> |

The build pipeline copies the licenses shipped with the pinned Node.js archive and npm dependency tree into the packaged runtime. Those upstream license texts govern their respective components.

The managed plugins can access capabilities exposed by the DSH Host. In particular, Better Sidebar can read and write workspace files, invoke Git, and create local PTY processes; Skin Center can update `$DSH_HOME/cordis.patch.yml`; GenUI actions send user interaction data back to the active model. DSH Desktop disables plugin install scripts and initializes Better Sidebar HTTP/HTTPS interception to off for a first managed installation.
