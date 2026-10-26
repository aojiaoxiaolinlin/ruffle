# Windows 游玩录制与内存对照

**本轮洛克“切几张地图后持续对战”的测试请先看 [Windows 交接与测试策略](WINDOWS_HANDOFF.md)。**
默认 `-Mode observe`：F8 开始；F9 依次标记切图、连续对战、停止后等待；F7 标记一场结束；F10 结束。
观察模式不记录普通输入、不注入操作，F9 不暂停计时。报告按四个阶段给出内存及对战后半段增长趋势。
下面的录制与回放模式用于需要复现输入的后续专项测试，本轮不要求使用。

独立工具，可用于现有 Windows 桌面程序，不需要重新编译游戏。需要 Windows 10/11、**64 位 Windows PowerShell 5.1**。分析报告另需 Python 3（可在 WSL 运行）。不支持 PowerShell 7 的 Add-Type/.NET 组合。

只采集指定 PID 的内存；录制模式的输入仅在 F8 开始后、指定游戏窗口获得焦点时记录。不启动游戏、不处理登录或保存票据。录制中的按键和检查点截图会保存到本地，因此应在登录完成后开始，不在录制期间输入密码或聊天。

## 统一口径

| CSV 列 | 含义 | 用途 |
| --- | --- | --- |
| `private_commit_bytes` | `GetProcessMemoryInfo.PrivateUsage`，该进程私有提交 | 主要比较指标；并非当前实际驻留 RAM |
| `working_set_bytes` | 当前驻留工作集，含共享页面 | 观察实际驻留变化 |
| `private_working_set_bytes` | Windows Process / Working Set - Private | 私有驻留 RAM；接近任务管理器某些默认“内存”列 |
| `gpu_dedicated_bytes` | Windows GPU Process Memory / Dedicated Usage，按 PID 汇总实例 | 单列比较 GPU 专用内存 |
| `gpu_shared_bytes` | 同一计数器组的 Shared Usage | 单列比较 GPU 共享内存 |

原始 CSV 单位是字节，报告统一为 MiB（除以 1,048,576）。**这些列不能相加**，尤其不能把 GPU 共享内存再加到进程内存上。当系统/驱动不提供计数器或该 PID 没有实例时留空，报告显示 N/A，不当作 0。Windows 计数器也不等同于 wgpu 的 allocated/reserved 或专用显存驻留量。

采样约每秒一次，由独立线程执行，记录 UTC、单调时钟、操作时间、检查点编号和采样耗时。不同计数器依次读取，不是原子快照。采样只覆盖游戏主进程，不含 WebView2 等子进程，工具自身的内存也不计入。每轮保存 EXE 路径和 SHA256，避免混用构建。

## 第一次：正常游玩并录制

打开 Windows PowerShell，进入本目录，游戏只保留一个实例。运行：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\Start-MemorySession.ps1 -Mode record
```

若无法自动识别，在任务管理器“详细信息”查看游戏 PID，增加 `-ProcessId 12345`。程序和工具保持相同权限等级。无需以管理员身份运行。

1. 手动登录，在起始地图等加载稳定，设置固定的窗口大小、画质和渲染后端。
2. 让游戏窗口处于前台，松开其他按键和鼠标，按 **F8**。工具保存起点截图并开始记录鼠标移动、左右中键、滚轮和普通键盘按键。
3. 正常游玩。到达需要核对的节点（例如地图加载完、进入战斗、战斗结束、回到原地图），松开按键，按 **F9**：保存检查点截图并暂停操作计时。核对画面后 **F8** 继续。等待期间内存采样继续。
4. 回到原地图后，可以静置 30～60 秒再结束，以留下稳定阶段。按 **F10** 结束。终端显示输出目录。

开始前的登录和等待不录入操作。默认输出到本目录 `sessions/memory-record-时间/`，已被 git 忽略；可以用 `-OutputDirectory C:\...\新目录` 指定位置。输出目录必须不存在，防止覆盖旧记录。

测试过程中保持鼠标在游戏客户区内，窗口留在屏幕内且不被其他窗口遮挡；不调整窗口大小或显示缩放。工具预留 F8/F9/F10，不支持 Win/Alt 系统组合键、侧键、手柄及 IME 中文组合输入。需要文字输入的测试另行处理。失焦、移出客户区或窗口尺寸/DPI 改变会使录制无效；F10 结束前需松开其他按键。

## 之后：重复回放

重启待测构建，手动登录、回到同一起点，恢复相同角色状态、打开的面板、窗口客户区尺寸、DPI、画质、后端和缓存条件。然后运行：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\Start-MemorySession.ps1 `
  -Mode replay -Recording .\sessions\memory-record-时间\recording.json
```

前台游戏中按 **F8** 开始，之后不要操作鼠标键盘。工具按原始时间顺序注入 Windows 输入，经过桌面程序正常输入路径。

- 默认到检查点暂停并截图：确认到达与录制相同的场景后 F8 继续。不需要手动重复移动和点击。**若场景已走偏，用 F10 终止，不要靠手动补点击“修复”这一轮。**
- `-Unattended` 会自动越过检查点并继续，供已经验证过的路线批量回放；结果标记为 `unreviewed`，需事后核对起点、检查点及终点截图。
- **F10 随时停止**。失焦、窗口尺寸/DPI 改变、注入失败或输入调度落后超过 250 ms 自动中止并标记无效。回放期间目标窗口中的人工输入会被拦截，F10 保留。
- 播完原始录制时长自动结束，保存终点截图。`complete=True` 只表示输入流程完成，不能证明游戏业务结果一致。

这是输入回放，不是服务器、随机数和网络响应的确定性重放。检查点可等待加载和核对场景，但不会自动识图找按钮、恢复不同的战斗结果。录制时在加载操作后留出充足等待，再做下一次点击；优先选择能回到同一起点、少依赖随机结果的路线。

第一次先做 30～60 秒短路线，确认洛克窗口的点击与坐标能正确回放，再录制正式路线。后续要比较基线/候选，**两边都回放同一份 recording.json**；首次人工录制主要用来制作路线。建议每个构建至少三轮，区分系统波动和真实变化。

## 报告与核心诊断对齐

```powershell
python .\summarize.py .\sessions\一次回放 --output .\one-run.md
python .\summarize.py .\sessions\baseline回放 .\sessions\candidate回放 --output .\comparison.md
```

报告按检查点分段，给出中位数、峰值和末 10 秒中位数；检查点等待单列，不计入操作阶段。两个输入记录不同或任一轮无效时拒绝给出比较差值。短于采样间隔的阶段可能没有数据；缺失项显示 N/A。检查点停留不同仍会影响加载、GC 和后续内存，报告会列出等待时长，不能仅凭过滤等待样本消除影响。

工具无需 `performance_diagnostics` 即可运行。如果需要把进程曲线与 SWF、脚本、位图、VTable、GPU 资源对应，基线和候选都启用该功能，并使用独立 `--cache-directory`：

```powershell
cargo build -p ruffle_desktop --release --features performance_diagnostics --locked
$env:RUST_LOG = 'warn,ruffle_desktop=info,ruffle_core::performance_diagnostics=info,ruffle_render_wgpu=info'
```

按 CSV 的 UTC 与已有诊断日志时间戳对齐。额外保留画质、后端、测试起点和路线说明；EXE 哈希不能代替这些配置。原始游戏日志可能包含登录信息，只提取需要的诊断数值。

## 工具验证

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\Start-MemorySession.ps1 -ValidateOnly
python -m unittest discover -s . -p 'test_*.py'
```

`-ValidateOnly` 编译 C#，检查输入记录验证、结构体 ABI、多显示器坐标转换、GPU PID 汇总与本进程 Windows 内存计数器，不安装钩子或注入输入。

`-SmokeTest -OutputDirectory C:\...\新目录` 会打开临时测试窗口并向它回放输入，验证正常鼠标/键盘消息、检查点、截图和 CSV。请在桌面空闲时运行。2026-10-26 在 Windows PowerShell 5.1 中通过自检与回放烟雾测试；首次回放烟雾测试最大调度延迟 18.6 ms。测试进程没有 GPU 计数实例，确认报告为 N/A。尚未完成洛克实机录制/回放验证，不能把工具烟雾测试等同于游戏路线验证。
