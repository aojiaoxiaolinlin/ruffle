# 洛克王国桌面登录

目前登录窗口仅在 Windows 提供，需要 Microsoft Edge WebView2 Runtime。

运行 `cargo run -p ruffle_desktop`，选择 **文件 → 洛克王国：进入游戏**。首次在腾讯官方页面完成 QQ 登录，成功后自动保存票据并启动游戏；以后进入游戏直接复用票据。关闭登录窗口可以取消。切换账号或登录失效时，选择 **文件 → 切换账号 / 登录失效，重新登录 → 洛克王国…**。

## 实现方式

- Wry/WebView2 使用独立的 InPrivate 登录会话。
- 官方页面自行处理 OAuth、授权码兑换、Cookie 和 iframe 跳转；应用不接管 QQ 密码或重复兑换授权码。
- 文档初始化脚本在游戏结果页观察 object/embed 的 FlashVars。iframe 通过受来源检查的 postMessage 转交顶层页面，顶层再通过 IPC 交给 Rust。
- 夜间保护时段官方会返回 `night.swf` 公告页而不是票据。客户端把该页视为登录流程的终止页：关闭登录窗口，并在主窗口用 Ruffle 播放官方 `https://res.17roco.qq.com/swf/night.swf`，不保存会话。
- Rust 再校验 IPC 来源、参数完整性和长度，固定资源配置地址，将参数直接交给现有播放器。
- 复用原生 TCP，允许目录服务器返回的游戏服地址。只有洛克 main.swf 使用专用 ExternalInterface。
- 最终 FlashVars 使用当前 Windows 用户的 DPAPI 加密保存，下次直接复用。不保存密码，不设置猜测的本地过期时间。票据不写入 SWF URL 或日志，导出 Bundle 时去除票据。

会话保存与失效处理的完整说明见 [GAME_LOGINS.md](GAME_LOGINS.md)。

## 验证

```powershell
cargo check -p ruffle_desktop --locked
cargo test -p ruffle_desktop roco:: --locked
node --test desktop/tests/roco-login.test.cjs
```

需要真实账号手工验证：扫码/授权、自动切换播放器、选服、进入可操作场景、取消后重新登录、重复点击登录不会多开窗口。自动化测试不代表已验证游戏完整可玩。

如果腾讯结果页更换 DOM 或不再生成 FlashVars，桥接需要调整。其他桌面系统暂不显示登录入口。
