# 洛克桌面内存排查：Windows 交接与下一轮策略

更新时间：2026-10-26。测试分支：`codex/roco-memory-investigation`，从 `aojiao-game-box` 的 `752b47e56` 建立，推送目标为 `aojiaoxiaolinlin/ruffle`（本机远端名 `fork`）。

## 这次要回答的问题

用户的实际负载是：**跳转几张地图，然后长时间连续对战**，桌面端观察到约 5～6 GB。尚未确定原观察值具体是私有提交还是某类工作集。本轮先统一采样口径，判断“加载后高位稳定”还是“重复同类战斗仍持续增长”，不要求复刻鼠标操作，也不急着比较新优化。

历史 VTable 优化只测得属性载荷下界约 180.7 → 112.0 MiB，不足以解释几 GB。现有核心资源计数也不是完整原生堆分析。仍需一次真实 Windows 游玩数据；工具自检不能代替这次测试。

## 在 Windows 取回工具

如果 Windows 的仓库已经有指向 `https://github.com/aojiaoxiaolinlin/ruffle.git` 的远端，使用该远端名。以下以 `fork` 为例，在仓库根目录运行：

```powershell
git fetch fork
git switch --track fork/codex/roco-memory-investigation
```

若本地已经存在同名分支，则 `git switch codex/roco-memory-investigation` 后 `git pull --ff-only`。不要覆盖 Windows 机器上原有的未提交改动。新电脑没有仓库时：

```powershell
git clone --branch codex/roco-memory-investigation https://github.com/aojiaoxiaolinlin/ruffle.git
cd ruffle
```

**工具和文档由 Git 同步；EXE、登录状态、录制数据和本地测试日志不会随 Git 同步。** 可先用 Windows 已有 EXE 采集操作系统内存；工具会记录其 SHA256。若需要同时解释内存归属，建议用下面的诊断构建。

## 推荐：一次采集同时保留核心资源日志

在已有 Windows Rust 构建环境中，于仓库根目录运行：

```powershell
cargo build -p ruffle_desktop --release --locked --features performance_diagnostics
$env:RUST_LOG = 'warn,ruffle_desktop=info,ruffle_core::performance_diagnostics=info,ruffle_render_wgpu=info'
$runCache = Join-Path $PWD ('desktop\tools\memory-session\sessions\game-log-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
.\target\release\ruffle_desktop.exe --cache-directory $runCache
```

此轮不额外启用 `vtable_memory_experiment`、XML 或 GPU memory policy 实验。若必须使用已有构建，在 `notes.md` 标明实际情况。原配置与登录过程照常；`--cache-directory` 用于隔离日志，**不证明所有游戏资源缓存都已冷启动**。无需清空现有票据或缓存。

在另一个 **Windows 本机 64 位 Windows PowerShell 5.1** 窗口启动采样，不从 Linux/WSL 弹出游戏或回放窗口：

```powershell
cd desktop\tools\memory-session
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\Start-MemorySession.ps1 -ValidateOnly
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\Start-MemorySession.ps1 -Mode observe
```

工具会自动选择唯一的游戏窗口；无法选择时在任务管理器“详细信息”找到 PID，追加 `-ProcessId 12345`。终端会打印输出目录。

## 首轮操作：约 20～25 分钟

先完成登录，固定画质、窗口大小、显示缩放、后端，游戏保持前台。观察模式只处理控制热键，不安装鼠标钩子、不存普通按键、不注入操作；按平时习惯游玩即可。

| 阶段 | 快捷键与操作 | 目的 |
| --- | --- | --- |
| 0 起点 | 在固定地图按 **F8** 开始，静置约 60 秒 | 起始私有提交/工作集 |
| 1 切图 | **F9 一次**，依次走平时的几张地图；记下顺序 | 加载资源后的变化 |
| 2 连续对战 | **再按 F9 一次**，在固定区域连续打 15～20 分钟 | 看后半段稳定占用和增长趋势 |
| 3 停止与等待 | 最后一场结束，**再按 F9 一次**；回固定地图，等待约 2 分钟 | 停止对战后仍保留多少 |
| 结束 | 保持游戏前台，按 **F10** | 写完结果并停止采样 |

F9 在观察模式中**只标记下一个阶段，不暂停计时、不截图、不改变游戏**。第 3 阶段之后再次按 F9 无效，避免误增阶段。第 3 阶段包含返回地图的过程，报告末 10 秒主要用于观察最终等待后的占用。

阶段 2 中每场结束可按 **F7**，不中断操作，用于记录完成场次。不是必需；漏标时在 `notes.md` 写明，不能把标记数当真实总场次。如果有余力，固定怪物种类、出战宠物、技能组合，减少负载差异；不要求每次操作时间完全相同。

观察模式预留 F7/F8/F9/F10；开始后不要切换窗口、最小化、调尺寸或 DPI。失焦和窗口变化会标记本轮无效。若出现错误或误操作，不必删除数据，保留结果并说明。回放工具的 F9 是暂停检查点，**不要与 observe 的连续阶段标记混用**。

## 输出、报告和下一步判断

默认目录是 `desktop/tools/memory-session/sessions/memory-observe-时间/`，包括：

- `memory.csv`：约每秒一次，五类内存分别以字节保存，带 UTC、操作时间和阶段编号。
- `timeline.csv`：开始、阶段变更、人工战斗结束标记和结束原因。
- `build.txt`、`session.json`、`result.txt`：EXE 哈希、窗口尺寸/DPI、采样完成状态；观察模式没有可回放的 `recording.json`。
- `start.png`、`end.png`：起止截图。`notes.md`：自动复制的测试说明模板，测试后补齐。

有 Python 3 时生成报告（也可把目录交给下一次排查，由 Linux/WSL 分析）：

```powershell
python .\summarize.py .\sessions\memory-observe-时间 --output .\sessions\memory-observe-时间\report.md
```

主指标为**私有提交**；工作集、私有工作集和 GPU 专用/共享内存单列，统一 MiB，不相加。缺失 GPU 计数显示 N/A。报告给出各阶段私有提交中位数、后半段中位数、后半段线性趋势 MiB/min，以及最终等待相比起点的差值。少于 60 秒的趋势覆盖不计算斜率；线性趋势是描述统计，不是泄漏判定或显著性检验。

- 连续对战后半段基本稳定：优先用同时间核心诊断或堆分析解释高位内存的大头。
- 同类战斗仍持续增长：对齐 SWF 数、GC 对象、像素/ByteArray/VTable 与 GPU 分配趋势，定位一直累积的类别，再决定抓原生堆分配调用栈。
- 结束后不下降：仍可能是有效资源缓存、分配器保留或尚未清扫对象，不能直接称为泄漏。

这一轮先定位趋势。后续有明确候选改动时，再在**同一台 Windows 电脑**按相同地图路线、相近怪物/宠物/技能和对战时长进行多轮对照；能记录场次则同时对齐。不同电脑的绝对内存不能直接作为优化收益。观察报告不自动输出“相同输入 A/B 收益”，因为没有记录确定输入；需要结合负载说明人工比较。

## 保存与验证状态

测试数据在 `sessions/` 中，已被 Git 忽略；提交测试分支不会上传截图、按键、票据或原始游戏日志。换电脑前另行保存需要的结果目录和诊断日志，游戏原始日志可能含敏感内容，公开分享前只提取诊断项。

截至交接：早期工具通过 Windows PowerShell 5.1 编译、Windows 内存计数器自检以及一次临时窗口回放烟雾测试（最大调度延迟 18.6 ms）。之后 DPI 修正通过只读自检，最终 GUI 复测被用户中止，未声称完成。观察模式是按连续对战策略追加的功能，仍需本次真实 Windows 热键/阶段采集验证。测试进程没有 GPU 实例，因此尚未验证真实游戏 GPU 数值，只验证了缺失值不会当成 0。

此分支是测试排查工具与策略，未修改 Ruffle 的 GC、输入处理、画质或内存优化默认开关。

本次交接提交前，含观察模式的最终 C# 通过 Windows 编译与只读计数器自检；Python 报告的 5 项测试通过，覆盖缺失计数、时间窗口、后半段斜率、观察模式结果加载及拒绝错误 A/B 比较。尚待 Windows 实际游玩验证热键、阶段标记与游戏 GPU 采样。
