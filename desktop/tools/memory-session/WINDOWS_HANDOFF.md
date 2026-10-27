# 洛克桌面内存排查：Windows 交接与下一轮策略

更新时间：2026-10-27。测试分支：`codex/roco-memory-investigation`，从 `aojiao-game-box` 的 `752b47e56` 建立，推送目标为 `aojiaoxiaolinlin/ruffle`（本机远端名 `fork`）。

**接续工作先读 [CURRENT_STATE.md](CURRENT_STATE.md)。** 实机首轮已经完成，离线堆归属流程也已实测通过；根因仍未确定。以下首轮 20–25 分钟操作属于历史方案，不应再让用户重复执行。用户白天在另一台电脑工作，Windows 本机实测留到晚上；白天优先开发快照差分和短时 PID 采集工具。

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

观察模式预留 F7/F8/F9/F10；切换窗口或最小化会继续采样，并在 CSV 标记后台时段，切回游戏自动恢复。报告排除后台样本，发生过后台采样的阶段不计算增长斜率；后台运行也可能影响后续占用，趋势测试尽量保持前台。F9/F7 在游戏前台使用，F10 可在其他窗口结束（后台结束不保存结束截图）。不要改变窗口尺寸或 DPI，窗口关闭仍会结束采样。若出现错误或误操作，不必删除数据，保留结果并说明。录制/回放仍在失焦时终止；回放工具的 F9 是暂停检查点，**不要与 observe 的连续阶段标记混用**。

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

## 2026-10-26 晚间实测补充：卡布西游

两轮卡布实测均正常结束。第二轮使用独立诊断构建，数据目录为本机 sessions/resource-diagnostic-20261026-224430-786。对战约 21.6 分钟，私有提交首末 10 秒前台中位数从 2813.2 增至 4726.5 MiB；等待约 2.9 分钟后为 4768.2 MiB。后台切换已继续采样并单独标记。

同期 SWF 数约 343 → 2216，角色定义约 1.57 万 → 8.23 万，GPU shape 约 5296 → 21233，shape 缓冲估算 78.6 → 348.4 MiB，上传位图句柄像素估算 144.4 → 278.4 MiB。临时纹理池反而略减，ByteArray 和已计数 CPU 像素容量基本稳定。归属估算重叠且不完整，不相加，不能据此解释全部私有提交。

离线测试 registered_symbol_retains_movie_until_library_is_dropped 确认库可通过注册 MovieClip 的共享定义间接持有强 SWF 引用，多轮 GC 后仍存活；去掉库根才释放。原空库测试及此诊断基线测试均通过。弱库键/弱 swf 字段不能独自打断这条引用链。下一步优先研究 SWF 库、角色定义和类的生命周期；尚未实现卸载修复，不能凭 Arc 强引用计数直接删库，也不能宣称所有旧 SWF 均为无用资源或全部增量都是泄漏。详细本地报告见该会话 memory/analysis-zh.md、resource-report.md 和 resources.csv。

### 后续已实现的独立候选

完整库所有权修复仍未实现，原因是库还持有应用域、类绑定及延迟实例化所需定义，直接弱化或删除可能改变脚本语义。先接通已有可重建渲染缓存的空闲回收，以验证实测网格及上传位图增长中的缓存贡献。仅 performance_diagnostics 构建读取 RUFFLE_DIAGNOSTIC_SWF_CACHE_IDLE_SECONDS；默认或 0 禁用。入口增加 -RenderCacheIdleSeconds 90，显式设置子进程策略并记录到 manifest、采样 build.txt 和报告。普通构建不读取此变量。

候选每五秒扫描一次，回收至少 90 秒未使用的 shape 网格、morph 插值帧及可释放位图填充句柄；保留 SWF、角色、导出名、类和脚本，按需重建。SWF render cache sweep 输出每次扫描的释放数；core.render-cache-sweep 计时，不能把释放数当作存活数或内存收益。需要观察扫描/重新绘制是否引入卡顿。

新增离线集成验证覆盖默认不回收、扫描间隔、共享实例、在途绘制句柄、导出名延迟实例化以及形变帧精确重建。核心单元测试 142 通过、1 个原有手动实验忽略；普通桌面 cargo check 和 Python 工具 10 项测试通过。此候选不是 SWF 卸载修复，尚无实机节省量或兼容性结论。下一轮应与同机上一轮对齐路线、怪物和技能，重点看 shape/上传位图/GPU allocated 是否趋稳，以及回旧场景是否正常；结束至少等三分钟。

### 2026-10-27 凌晨候选实测

首轮候选被用户报告掉线并关闭，目录尾缀 20261026-234911-451，complete=False，不作完整对照。重测目录为 resource-diagnostic-idle-render-cache-candidate-20261027-000150-146，正常结束，用户确认无明显卡顿、最后一段已停止对战。末段未标 F9 cooldown，且约三分钟后台，不能将机器标记阶段 2 全部解释成持续战斗；原 CSV 不改写。

取前台对战相对时间 14.5—15 分钟窗口：基线/候选私有提交为 4175.4/4033.6 MiB，网格缓冲估算 233.1/11.0 MiB，上传位图句柄像素估算 202.1/21.2 MiB，GPU allocated 688.60/277.66 MiB；reserved 均为 1536 MiB，Windows 专用 GPU 计数约 1531/1529 MiB。负载和角色定义数量不同，不宣称精确 A/B 百分比收益。

候选自身新 SWF 仍增加时，网格和上传位图估算明显下降，策略与扫描计时日志确认已启用，支持回收生效。246 次扫描平均约 1.05 ms、最大 3.71 ms，无单次扫描超过 42 ms。候选私有提交从对战开始约 3178.3 MiB 增至约 15 分钟 4033.6 MiB，最终等待后约 4158.1 MiB；SWF 最终 1392 个，完整库/脚本生命周期问题未解决。

本轮漏采逐次释放数，原因为过滤器缺少 ruffle_core::library=info，已修正启动器；不补造历史记录。下一步优先补齐 GC 字节及 CPU 原生缓冲归属，沿 SWF/类/应用域引用关系排查，同时区分 GPU reserved 保留行为。候选仍默认关闭。详细限制和数据见该会话 memory/analysis-zh.md。


### 2026-10-27：停止重复手动对战，转向离线统计和堆归属

用户明确表示反复长时间测试负担过大。当前不要再让用户启动 20 分钟观察轮次。

本轮核对 Cargo.lock 和实际编译 dep-info，确认使用 registry gc-arena 0.7.0，而非本机 git checkout 的旧版本。**纠正前一轮解释：gc_debt 是对象数量驱动的 pacing 工作量，不是字节**。0.7.0 不提供 total_gc_allocation / total_external_allocation；本轮未伪造 GC 堆字节指标，未修改依赖或 GC 策略。历史日志中的 byte-units 说明错误，报告解析器统一改为 debt units。

已补充 performance_diagnostics 专用 dynamic_map_tables_bytes（HashTable allocation_size，含桶和控制区）及 vector_capacity_bytes（Vec capacity × Value 大小），支持 created/dropped/live。均不持有对象引用，不含引用的其他 payload。动态表的删除不会收缩容量；Vector 清空并不一定释放，克隆容量需重算。新增离线测试覆盖这些路径，以及 DynamicMap 的 GC 弱引用回收。Array 的稠密/稀疏存储仍是盲区，当前没有证据认定新增两类就是实际游戏的大头。

本机只读工具检查：System32/wpr.exe 支持 Heap、HeapSnapshot、VirtualAllocation profiles；Windows Performance Toolkit 中 wpa.exe 和 wpaexporter.exe 可用（exporter 需脱离沙箱访问其注册表配置）。现有诊断 exe 旁有 PDB，但未验证具体函数/行号解析。当前 token 非管理员，未启动 ETW、未更改 IFEO、未触碰游戏。下一步堆采集需先在自建离线样例上验证：独立命名的 WPR session、仅样例 PID 的 heap snapshot、基线/分配后/释放后三个快照、EXE/PDB 与 trace 配对、WPA 导出分配栈并核验已知分配尺寸。通过后才考虑短实机采集；不能仅因 PDB 存在就声称符号流程跑通。

本轮验证：核心 release diagnostics 测试 145 passed / 1 ignored；Python 报告测试 11 passed；普通 desktop cargo check 和 diagnostics release build 均成功。历史两轮 resource-report/resources.csv 已重新生成以修正 gc_debt 单位，未修改原始日志。新诊断 exe SHA256：`a4464fff47df41c2dd779b2e07b8e45b2ead9db79aafb4dfe1b1b4180f6ab4be`。没有启动新游戏测试。


### 离线堆归属验证脚本已准备（2026-10-27）

新增 `heap_fixture.rs` 和 `Test-HeapAttribution.ps1`。无需 Cargo 依赖，rustc 生成 EXE/PDB。样例 payload：0 → 18 MiB（64×256 KiB transient + 8×256 KiB retained）→ 2 MiB → 0。ValidateOnly 已成功；Rust backtrace 能解析 symbol_probe_site 和源码文件。另以隐藏进程实测四阶段文件控制协议和成功退出。最终验证产物在 `target/heap-attribution/20261027-010602-251`，包含 EXE/PDB/source、哈希、期望值、符号探针、协议测试输出。没有游戏或网络操作。

实际尝试 `wpr -start HeapSnapshot -filemode -instancename RuffleHeapPreflight` 返回 0x80070005 Access is denied；这是 Windows token 未提升，不是自动审批拒绝。没有成功开启采集会话。完整脚本需用户在管理员 PowerShell 中运行（不带 ValidateOnly），自动对样例 PID 做三次快照并保存 ETL，使用独立 WPR 名称且不改持久 IFEO。失败时仍尝试保存本会话 trace、禁用该 PID 配置并退出样例；stop_failed 会记录 session 名。尚未执行管理员采集分支，故 HeapSnapshot/WPA 符号归属仍未验证。拿到该离线 ETL 后先核对 transient_allocation_site 的 16 MiB 释放和 retained_allocation_site 的 2 MiB 保留，再考虑游戏采集。不要把样例自报 payload 当作进程 commit，也不要把 Rust backtrace 符号成功当作 WPA 栈解析成功。


### 真实离线 ETL 验证通过（用户提供 20261027-010929-552）

完整 WPR 三快照均成功，PID26252，事件/缓冲丢失均0。新增 Export-HeapSnapshots.ps1 使用 xperf -symbols -a heapsnapshot -instance -data -stack -count，设置并恢复本进程本地符号和 symcache 路径。无需操作 WPA GUI。新增 summarize_heap.py 解析各快照分配记录，验证引用数/大小之和/声明快照及分配数，忽略尾部全局栈表，输出 heap-summary.json；--verify-fixture 严格要求两个已解析函数的块数为 (0,0)/(8,64)/(8,0)。

实际堆：baseline14项7025字节；allocated88项18883121字节；released23项2104369字节。两个 Rust 分配函数名均解析成功；64×256KiB transient（16MiB）在释放后消失；8×256KiB retained（2MiB）保留。验证到函数名，未验证行号，系统 DLL 仍可显示地址。已实跑导出脚本和自动核验通过；Python全部15测试通过。中文结论存该目录 analysis-zh.md。

现在采集/应用函数归属链路具备实证。游戏根因仍未知；下一次实机采集须在关注的分配发生前启用跟踪，按快照差分后的保留栈定位，不再要求用户反复20分钟纯总量观察。Windows heap 以外的直接 VirtualAlloc/GPU 仍需另行对照。


### 晚间实机三快照与 LoaderInfo 修复候选

完整洛克采集 `target/game-heap-capture/20261027-210135-954`：PID19232，21:03:26基线、21:11:20对战后、21:13:24冷却120秒后。全部保存，事件/缓冲丢失0。Windows heap 1164.09→2549.39→2413.30MiB。Vulkan驱动、事件、显示对象及类表是较大的来源；GC确实执行，但仍存在新一轮垃圾，不能按分配栈直接判定根因。原始ETL/日志/EXE/PDB及大文件导出仍在本机ignored目录。

沿来源做离线引用对照，确认 Library根SymbolClass原型 → 共享LoaderInfo → loaded_stream的原根内容 → 子树 会保持旧内容。修复36bfdd56c将LoaderInfo移到MovieClip实例，并在符号克隆中不继承原加载上下文。对照测试验证旧子树可回收、显式保留LoaderInfo时内容仍正常保留、符号晚实例化继续有效。Loader/根影片/SymbolClass/大预加载回归均通过；核心146 passed/1 ignored，Python20 passed，普通check和诊断build通过。候选尚无新实机量化，不宣称全部5GB已经解释或修复。当前工作以CURRENT_STATE顶部和LOADER_INFO_FIX为准，不再继续历史长时间纯观察。
