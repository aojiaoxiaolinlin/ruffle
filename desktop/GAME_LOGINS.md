# 桌面游戏登录

同一个 Ruffle 桌面程序提供两个入口：

| 菜单 | 登录方式 | 游戏入口 |
| --- | --- | --- |
| 文件 → 洛克王国：进入游戏 | 优先使用保存的票据；首次在 WebView2 中完成官方 QQ 登录 | `https://res.17roco.qq.com/main.swf` |
| 文件 → 卡布西游：进入游戏 | 优先使用保存的票据；首次输入4399账号密码 | `http://enter.wanwan4399.com/bin-debug/GreenGame.swf` |

两种登录都把票据通过 FlashVars 传给现有播放器，在当前游戏窗口切换游戏。安卓工程不作修改。

## 保持登录与切换账号

- Windows 使用当前用户的 DPAPI 加密保存最终游戏票据，文件位于 Ruffle 配置目录的 `game-sessions/roco.bin` 和 `game-sessions/kabu.bin`。密码、浏览器 Cookie 和登录响应不保存。
- 更新到此版本后，两款游戏各完成一次登录；之后关闭游戏或重启程序，再点击“进入游戏”直接复用保存的票据。“最近打开”和书签中的游戏入口也会尝试恢复会话。
- 不假设票据有效期，不因为经过一天、普通断线、服务器维护或 HTTP 资源错误而清除登录状态。票据能否继续使用由游戏服务器决定。
- **文件 → 切换账号 / 登录失效，重新登录 → 对应游戏**：清除该游戏的票据，关闭该游戏当前会话并重新登录；保留另一个游戏的票据。取消新登录后旧账号不会自动恢复。
- 已知的游戏导航回官方登录页会接回客户端重新登录。部分游戏认证失败只显示在 SWF 内，尚无通用的自动识别信号，这时使用上述菜单。普通网页刷新请求复用原票据。
- 读取到损坏、无法解密或格式错误的会话时重新登录；保存失败会明确提示，本次游戏仍可继续。
- 保存文件不能跨游戏互换，导出 Bundle 仍会剔除登录票据。非 Windows 平台暂未提供安全持久存储，绝不回退到明文。

## 卡布西游

输入4399账号密码，点击“登录并进入游戏”。支持取消、失败重试和登录域名备用切换。账号密码不保存到文件。当前快捷登录与安卓实现相同，不支持交互验证码；触发验证码时会提示失败。

协议参考用户的 `E:\kabu\rust\kabu-login` 和安卓 `origin/kabu`：

1. 获取4399登录页面与会话 Cookie。
2. 按 CryptoJS 的 OpenSSL 口令模式加密密码并提交表单。
3. 请求 `checkKidLoginUserCookie.do`，读取返回的跳转地址。
4. 校验跳转的域名、路径和 `sig`，保留完整原始查询串作为 `auth_string`。不单独取 `sig`，不对整串解码或重复编码。
5. 设置卡布的 SWF、base URL 和 TCP 连接选项；独立于洛克的 HTTPS 与 ExternalInterface 配置。

登录请求沿用已验证实现中的官方 HTTP 接口与资源地址。错误提示不包含原始服务器响应、密码、Cookie 或票据。关闭登录对话框会取消后台任务；切换游戏时会清理旧对话框。导出 Bundle 会剔除这两个游戏的登录票据。

洛克的具体实现见 [ROCO_LOGIN.md](ROCO_LOGIN.md)。

## 验证

```powershell
cargo check -p ruffle_desktop --locked
cargo test -p ruffle_desktop --locked
node --test desktop/tests/roco-login.test.cjs
cargo build -p ruffle_desktop --locked
```

测试覆盖加密存储、重启恢复、账号替换、跨游戏隔离、文件损坏，以及卡布的加密向量、原始票据保留、完整登录协议和失败处理。真实账号的票据过期行为仍需人工验证。
