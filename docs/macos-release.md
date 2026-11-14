# macOS 正式发布

公开分发使用付费 Apple Developer Program 的 **Developer ID Application** 证书及其私钥。Apple Development 仅用于本地开发，不能替代公开分发签名。应用标识固定为 `com.atat.lighttranslate.v2`，后续版本使用同一开发者团队，保持 TCC 授权所识别的应用身份稳定。

## 一次性配置

在 Apple Developer 的 Certificates 中创建 Developer ID Application 证书，将证书及对应私钥导出为带密码的 `.p12`。为 Apple 账号创建应用专用密码，用于公证。私钥、证书导出文件和密码均不得提交到仓库。

在仓库的 Actions secrets 中设置：

| Secret | 内容 |
| --- | --- |
| `APPLE_CERTIFICATE` | `.p12` 文件的 base64 内容 |
| `APPLE_CERTIFICATE_PASSWORD` | `.p12` 导出密码 |
| `APPLE_SIGNING_IDENTITY` | 完整身份名，如 `Developer ID Application: Your Name (TEAMID1234)` |
| `APPLE_TEAM_ID` | 与签名身份匹配的 10 位 Team ID |
| `APPLE_ID` | 开发者 Apple 账号 |
| `APPLE_PASSWORD` | Apple 账号的应用专用密码，非登录密码 |

Apple 账号如有待接受的新开发者协议，须由账号持有人先处理。证书和密码配置完整后才能发布；缺失时 CI 会失败，不会改用临时签名。

## 版本发布

1. 同步修改 `package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml` 与 Cargo.lock 中本项目的版本号。
2. 合并并通过常规 CI 后，在该提交上推送对应的 `v<版本号>` 标签。
3. 标签触发正式构建；普通分支与 PR 仅构建测试包，不接触签名凭据，也不创建公开 Release。

发布前用「Query macOS notarization」查询已经交给 Apple 的 submission ID。该工作流不编译、不上传、不发布。要对一份新的二进制签名并公证，需同时勾选 `sign_macos` 和 `submit_new`；只勾选 `sign_macos` 不会构建或上传。手动流程不会创建公开 Release。

macOS CI 使用临时钥匙串导入证书，由 Tauri 生成双架构应用并签名、启用 hardened runtime。Tauri 拿不到公证凭据。发布脚本对应用归档只上传一次，保存提交编号并查询结果；确认 Apple 接受后装订并验证应用票据。随后用系统 `hdiutil` 将已装订的应用和「应用程序」文件夹链接打包为 DMG，对 DMG 签名，再对 DMG 只上传一次并装订票据。构建结束会删除临时钥匙串。

发布前 `scripts/verify-macos-release.sh` 检查 DMG 和只读挂载后的应用：有效的 Apple Developer ID 证书链、指定团队、固定 bundle ID、不包含 cdhash 的 designated requirement、Intel/Apple Silicon 双架构、各架构的 hardened runtime 和可信时间戳、两层公证票据及 Gatekeeper 接受结果。任一步失败都会阻止发布。已存在的 Release 不覆盖，需使用新版本号。

首次公证可能等待较久。应用和 DMG 分别默认最多等待两小时，可通过 `LT_NOTARY_TIMEOUT` 调整。每次上传立即保存 Apple submission ID、文件 SHA256 和状态到 `*.notarization.json`。状态查询遇到网络断开或超时会继续查询同一个编号。上传没有返回编号时不会再上传。文件内容变化后旧回执不能用于新文件。只有 Apple 明确返回 `Accepted` 才会装订票据；票据检索最多重试五次。

客户端在 Apple 仍为 `In Progress` 时超时，或轮询时网络中断，都不是 Apple 拒绝。不要因此重新运行失败 job：新的 runner 会编出另一份二进制并再次上传。最近 12 小时内还有 `In Progress` 提交时，签名任务会在编译前停止。用「Query macOS notarization」查询原来的编号，包括 Apple 返回 `Invalid` 或 `Rejected` 时的公证日志。标签发布使用同一条限制。

超时或拒绝都会阻止发布。CI 保留签名应用 ZIP、DMG 和回执七天；`macos-notarization-recovery-*` artifact 不会被公开 Release 收集。手里仍有同一份文件和回执时，运行 `python3 scripts/notarize_macos.py <文件>` 只查询已有编号。必须完成票据装订及整个安装包验收后才能发布。

也可在 Mac 上对实际下载的文件独立验收：

```bash
bash scripts/verify-macos-release.sh /path/to/LightTranslate-macOS-Universal.dmg TEAMID1234
```

首次辅助功能与屏幕录制授权由用户在系统设置中完成。签名和公证不绕过系统授权。旧的未签名或开发版迁移到 Developer ID 版时，可能需要移除旧权限条目并重新添加一次；正式版仍需在干净用户账号或另一台 Mac 上验证真实授权流程。

参考：[Apple 公证要求](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)、[Tauri macOS 签名](https://v2.tauri.app/distribute/sign/macos/)。
