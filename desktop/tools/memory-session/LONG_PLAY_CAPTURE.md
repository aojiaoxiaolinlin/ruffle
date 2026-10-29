# 长时间正常游玩的诊断记录

## 先取回已有日志

生命周期版高占用截图对应的日志仍有分析价值，不需要先重测。
请测试者先复制那次程序的整个日志目录，并说明大致启动时间、游戏名称、
游玩时长、截图时间，以及截图前是否已停止对战。时间记不清可以直接注明未知。

程序把文件放在所用 cache-directory 下的 log 目录；默认文件名为 ruffle.log，
也可能配置成带时间的文件名。默认文件下次启动可能被覆盖，应先复制再启动。
如果通过采集脚本启动，优先保留整个对应 sessions 子目录。
若找不到目录，可在程序使用的缓存目录内搜索 ruffle*.log。

普通日志不一定启用了资源诊断；拿到文件后先检查，不根据截图推算影片数或 GC 状态。
日志可能包含登录票据和游戏请求，请私下传递，原始文件不提交仓库。

## 后续自动采集

使用私有分支最新的 performance_diagnostics Windows 成品及其配套文件。
需要同目录的 ruffle_desktop.pdb 和 build-identity.json，后者含 revision、
exe_sha256，以及 gc_work_floor_supported。沿用构建时生成的身份文件，不能为旧 EXE
手工声明支持实验。脚本核对 EXE 哈希；PDB 是否真正匹配仍依赖构建时配套保存。

在 64 位 Windows PowerShell 5.1 中运行：

```powershell
# 先只检查文件，不启动游戏
.\desktop\tools\memory-session\Start-AutomaticDiagnosticSession.ps1 -ExecutablePath 'D:\candidate\ruffle_desktop.exe' -ValidateOnly
# 生命周期版原有 GC 策略，正常游玩，自动记录到退出
.\desktop\tools\memory-session\Start-AutomaticDiagnosticSession.ps1 -ExecutablePath 'D:\candidate\ruffle_desktop.exe' -GcWorkFloor 0
# 另一次游玩才使用 GC 推进实验；需要确实包含该实现的新版 EXE
.\desktop\tools\memory-session\Start-AutomaticDiagnosticSession.ps1 -ExecutablePath 'D:\candidate\ruffle_desktop.exe' -GcWorkFloor 1
```

保持采集窗口打开，无需 F8/F9/F10，也不要求固定游玩时长。结束时正常关闭游戏。
如果自然停战或观察到高占用，顺手记下本机时间即可，不需要反复执行测试流程。
脚本不对正在运行的其他游戏进程采样，也不结束它们。
每轮使用独立缓存目录，首次加载可能与日常缓存命中情况不同。

文件自动保存在 EXE 同目录的 sessions/时间戳 下：

- session.json：EXE 哈希、版本、PID、GC 开关、开始时间。
- os-memory.csv：约每秒进程 private commit、工作集、虚拟地址空间。
- system-memory.csv：约每 5 秒系统提交量、提交上限及可用物理内存。
- stdout.log、stderr.log、game-cache/log：资源诊断、警告及可能的崩溃信息。
- result.json：结束状态、退出码、采样次数、采样捕获到的内存峰值。

请保留整个单次会话目录。正常退出会写结束摘要；采集异常时尽力写中断状态，
断电或强制关闭采集器可能没有摘要，已经刷新到磁盘的 CSV 仍有价值。
开启回溯不保证内存分配失败时能输出完整调用栈。

## 如何判断

### 同时检查流畅度

现有采集过滤器已包含以下日志，无需另外开启逐帧输出：

- Desktop perf（约每 5 秒）：render_hz 为重绘吞吐；tick/render/gui 有平均、P95 上界和最大耗时。
- Perf progress：frame_steps/window_s 可估算逻辑帧推进尝试频率，另有追帧、丢弃 tick、音频校正计数。
- Perf phase：core.frame-gap、core.tick-gap、desktop.redraw-gap 等间隔，以及 core.gc、core.frame-scripts、渲染等阶段耗时；含 over_42ms、over_83ms 次数。
- Perf stall：达到 83ms 的阶段/间隔即时记录，每统计窗口最多 12 条；总次数看阶段汇总，不能数 stall 行代替。

分析内存时必须同步检查这些字段：高占用前后是否间隔变长、长耗时次数增加，
GC 推进是否换来了更长的暂停。比较每个有效前台窗口的分布和长间隔次数，
不能只看全程平均频率。已有 summarize_resources.py 只做资源归属，不会自动给出卡顿判定，
需要同时读取上述性能日志。

render_hz 不是实际显示器呈现 FPS，frame_steps 是推进尝试而非保证成功的帧数。
音频 advance/delay 是同步校正幅度，不是网络延迟或输入延迟。
目前没有点击到画面响应的端到端延迟，也没有 GPU 实际完成/呈现计时。
42/83ms 是固定诊断阈值，不是所有游戏通用的卡顿判据；应结合游戏目标帧率、
前后台、最小化、暂停、加载阶段及用户感受解释。普通 24 FPS 的预期间隔约 41.7ms。
免按键采集没有自动前后台分类，不能把切出窗口后的长间隔直接判作前台卡顿。
嵌套阶段耗时会重叠，不能相加，也不能仅凭 GC 最大耗时就断言整个游戏不卡。

先对齐进程内存、影片/资源数量、GC 对象数量与回收阶段，再决定是否需要堆快照。
区分持续分配、合法保留、GC 推进速度及原生分配；不把系统提交、进程工作集和
GPU reserved 相加。不同游玩负载的两轮结果不能直接计算优化百分比。

2026-10-29：本机只准备了脚本和说明，未运行 Windows 长测，未构建新的 Windows EXE。
GC 实验已有短测不能替代长时间验证，也不能据此称整体高占用问题已解决。
