# 内存排查当前状态与跨电脑交接

更新时间：2026-10-28（Asia/Shanghai）。工作分支：`codex/roco-memory-investigation`；主交接仓库：私有 `aojiaoxiaolinlin/ruffle-A`（远端 `private`）；`fork` 为可删除的公开副本。

## 2026-10-29 交接：长测高占用属于生命周期版

详见 [LONG_PLAY_HANDOFF_20261029.md](LONG_PLAY_HANDOFF_20261029.md)，另一台机器请先读该文。用户明确确认：他人长时间游玩出现约 6–7 GB 的两张任务管理器截图，均来自生命周期优化版，**不是 GC 推进实验版**。目前没有取得对应原始日志，EXE 哈希也未核验，不能归因为 GC 实验失败。GC 版仅做过登录、对战几局和结束后的短测；此前末段稳定只适用于该观察窗口，不代表长测通过。

本次只补交接说明和长测采集工具，不修改生产 GC/生命周期策略，不包含本地未提交的 AVM 执行/内存基线。优先索取旧日志；如不可得，后续正常游玩时用自动脚本采集并记录 GC 开关。没有开展新的长测、没有生成新 Windows EXE。以下各节保留历史记录，状态以本节和交接文为准。

## 最新实机结果：GC回收与内存稳定观察

见 [GC_WORK_FLOOR_WINDOWS_RESULTS.md](GC_WORK_FLOOR_WINDOWS_RESULTS.md)。保留会话20261028-210105-649的回收及暂停数据：21:09–21:27末段工作集约1587–1596MiB、私有提交约3402–3414MiB、影片对象58。GC对象从约96万涨至140万后回落到约96万，活动周期缩短；不同负载不能计算精确收益。默认关闭状态保留，尚未完成全局启用评估。

新增自动启动采样脚本支持后续启动的Rust回溯、约5秒系统提交采样及退出码捕获，已作语法/ValidateOnly/exit17验证。正常长时间游玩仍有助于验证持续增长与暂停表现，不要求用户重复繁琐流程；后续有新的异常再针对当轮排查。

## 实机短测与默认关闭的GC推进实验

本机已构建d0eac7411生命周期版，核心167项通过。卡布会话20261028-195829-034停战120秒：影片对象256→139、网格70.8→47.2MiB、工作集1839→1730MiB、私有提交4148→4037MiB。随后闲置回升，但影片/网格不反弹；GC在低分配负载下标记约140秒，清扫持续到退出前。用户未观察到卡顿，现继续正常游玩，无需配合额外测试。

详见 [GC_PACING_ANALYSIS.md](GC_PACING_ANALYSIS.md)。已建立固定存活图的离线对照；活动期每50ms补至2000工作单位可降低该模型峰值，但增加CPU工作。新增播放器实验仅performance_diagnostics且RUFFLE_DIAGNOSTIC_GC_WORK_FLOOR=1启用，默认关闭，不唤醒休眠GC，不更改默认睡眠参数。两个节流/零分配推进回归及完整169项核心测试、普通桌面检查、诊断release构建、模型存活图完整性校验通过。尚未实机验证该实验，不解释全部进程内存；当前用户正在玩的d0eac7411独立EXE不受源码改动影响。

## 最新完成：全局弱索引已启用

生产MovieLibraries现为弱影片键 + 已trace的GC弱定义值，既不通过应用域也不通过符号原型自我保活。定义所有者强持有不可变影片；读查询使用Mutation升级弱句柄，失效条目不返回已销毁对象。字体、AVM1函数/排队动作/保留的PlaceObject事件、附加声音/播放实例已接入来源定义持有。导入字体和字符保留原来源，避免反向保活无用导入者。

最终核心诊断单元测试167 passed、0 failed、1 ignored。新增32轮AVM1实际SWF字节加载/替换/释放，以及16轮AS3 loadBytes完成回调/真实LoaderInfo.unload：每轮旧定义全部回收，索引数量有界。合成影片无真实游戏ABC/网络；不代表实机节省量。

16组预编译SWF回归均无失败，其中AVM1组795 passed/21 ignored；事件所有权补齐后最终又复查Loader、button、font、function、event、timeline、AVM1七组，均通过。分组有重叠，含既有预期差异和渲染/视频等忽略。普通及performance_diagnostics桌面cargo check、格式与diff检查通过。未构建新Windows EXE，旧候选EXE不能验证此轮弱索引实现。

用户要求本轮提交同时推送私有仓库与fork，并将后续交接优先放在私有仓库同名分支；默认分支0.7.1-A不作合并或覆盖。下方记录为分阶段历史，以此节和LIFETIME_OWNERSHIP_ANALYSIS.md的“全局弱索引落地”节为准。

## 最新推进：显示实例、在途加载与跨影片符号所有权

已接入显示对象基类的定义强句柄，包括库实例化、动态分配/post_instantiation、root/import创建、影片替换与Stage根影片更新；Class显式持有首次绑定的符号来源库；导入字符复制会登记来源依赖；MovieLoader在首次progress前持有真实定义。

新增4项离线生命周期回归覆盖脱离树的MovieClip/Graphic、跨库导入循环、父类绑定子影片符号、实际字节加载后目标替换。原类绑定测试也覆盖注册表重建后的重新绑定。核心诊断单元测试159 passed/1 ignored；11组预编译SWF回归无失败，普通桌面cargo check、格式及diff检查通过。全局索引仍强trace，尚未开启弱值；字体、AVM1动作/声音和其他共享定义消费者仍需继续审计。没有要求用户游玩，也未生成新Windows EXE。详细边界见LIFETIME_OWNERSHIP_ANALYSIS.md“生产迁移第二步”。

## 最新推进：生产库所有者与脚本定义已接通

生产影片库现存储GC管理的MovieDefinition；普通ABC和内置ABC的TranslationUnit必须持有对应所有者。新增隔离测试验证：移除全局索引后，外部Class仍能保持定义库和晚实例化有效；释放最后Class后定义环与影片可回收。库访问已迁移为带写屏障的Ref/RefMut，并缩短预加载、SymbolClass、导入与Loader路径的借用范围。

全局索引仍强trace库所有者，显示对象、在途加载和跨影片符号绑定等持有关系尚未全部迁移，不能据此声称实机内存问题已修复，也不能提前弱化全局索引。核心诊断单元测试155 passed/1 ignored；Loader、SymbolClass、root、导入、按钮、文本、声音、应用域、预加载、attachMovie和位图11组SWF回归均无失败（组间重叠，含既有预期差异和环境忽略）。普通桌面cargo check、格式及diff检查通过。详细实现边界与分组计数见LIFETIME_OWNERSHIP_ANALYSIS.md的“生产迁移第一步”。未生成新Windows EXE、未安排用户游玩。

## 本机继续：外部定义所有权的离线对照

已增加36轮实际Loader卸载对照（不保留定义/保留Class/保留Domain各12轮），以及两份影片共享域、仅外部保留一个Class的对照。前者检查内容树释放及最后一个定义持有者释放后的回收，后者检查共享域内其他影片的合法保留。源码消费者落点与迁移注意事项已补入LIFETIME_OWNERSHIP_ANALYSIS.md。没有要求用户游玩；本轮没有修改生产定义生命周期或生成新游戏EXE。另已完成测试内的GC库所有者/弱索引原型，验证跨库依赖环可在最后一个消费者释放后回收，且存活时导入符号能晚实例化。核心诊断单元测试154 passed/1 ignored，格式及diff检查通过。原型尚未接入生产路径；详见该文后续记录。

## 最新分析：内容 unload 与定义回收是两层生命周期

新增真实Loader构造/LoaderInfo.unload/容器移除/GC的离线对照已通过：空Loader仍被强持有时，旧内容树可以回收；但影片库保存应用域时，影片定义依旧存活。详见 [LIFETIME_OWNERSHIP_ANALYSIS.md](LIFETIME_OWNERSHIP_ANALYSIS.md)。影片和ABC为测试构造，未证明实机游戏调用过unload或所有旧定义都应卸载。

当前判断是全局索引桥接Arc影片和GC对象时存在自我保活关系，不是GC不会回收不可达环。后续应迁移至由实际实例/脚本/类持有的GC定义所有者及弱全局索引；不能单独弱化库值或清空共享域。此轮只增加回归与设计分析，生产EXE仍以以下类注册表候选为最新，没有启动新实机测试。

## 最新候选：类注册表不再强持有类定义

详见 [CLASS_REGISTRY_RETENTION.md](CLASS_REGISTRY_RETENTION.md)。已离线证明全局类注册表和影片库application domain各自可独立保活Class→Method→TranslationUnit→SWF。本次仅解除前者：使用正确trace的GcWeak身份，首次绑定优先及外部类仍存活时的查找保持有效。**库域/字符原型仍是独立强根，尚不能保证实机SWF减少或解决全部4–5GB。**

核心150 passed/1 ignored，Loader 52 passed/2 ignored，SymbolClass 6 passed，root 30 passed/1 ignored，application_domain 1 passed；组间有重叠并含既有已知差异校验。普通desktop cargo check、完整诊断release构建及格式检查通过。独立成品 `target/memory-candidates/weak-class-registry-20261028/ruffle_desktop.exe`，SHA256 `ccb073355f697dcb4abc46567e3113f0e046d39b385642e2b0d58deeab9cae3c`。未启动新实机测试。

事件候选的卡布长测（23:35:33至次日00:07:42）已保存最终日志，确认EXE哈希匹配。23:50→次日00:00的SWF中位数1516→2251、网格488→772MiB、GPU allocated1178→1620MiB；原地阶段精确边界未知，最后GPU allocated波动并有回落，不能从OS总量直接定性泄漏。XML查询计数/压缩及紧凑VTable均未启用；历史点击异常发生在compact=false的计数基线，原因未定。无需用户继续长时间手动游玩。

## 前一候选：跳过无人监听的显示列表事件构造

首轮 LoaderInfo 候选实机日志的分析见 [CANDIDATE_TEST_ANALYSIS.md](CANDIDATE_TEST_ANALYSIS.md)，后续具体优化见 [EVENT_ALLOCATION_FIX.md](EVENT_ALLOCATION_FIX.md)。四类内部显示列表事件仅在整条祖先链都无监听器时跳过构造；保留非冒泡事件的祖先捕获和递归子节点检查。离线1000轮四事件从新增8000个GC对象降至0，尚未证明实机节省量或解决全部内存增长。

新增两个离线回归；核心148 passed/1 ignored，预编译SWF的event组115 passed（含既有已知差异的预期校验）、simplebutton_added_to_stage 1 passed，普通desktop cargo check通过。新诊断EXE/PDB已完成链接并保存到 **`target/memory-candidates/event-listeners-20261027/ruffle_desktop.exe`**，EXE SHA256 `87a606d6382a31fa85f7f562acd8bd1bff3c86fcac03963b6fc387e806149149`。已验证EXE与PDB的CodeView GUID/age匹配、EXE --version正常退出。

构建命令最终复制到旧release根目录时遇到运行中EXE占用而非零退出；独立目录使用的是这次成功链接的release/deps成品。22:58:15重新启动的PID16732仍是旧路径下的LoaderInfo候选，未关闭或替换它，不能将它的后续表现当作新事件候选效果。首轮候选的原EXE/PDB已和日志一起备份，原始数据不入Git。

## 当前优先状态：已有解除保留链的修复候选

修复提交：`36bfdd56c`（fix(core): keep loader state out of cached movie symbols）。详见 [LOADER_INFO_FIX.md](LOADER_INFO_FIX.md)。LoaderInfo 已由 MovieClipShared 移到实际 MovieClipData 实例，符号实例化不再继承原加载的 LoaderInfo。已复现的“库原型 → LoaderInfo → 原加载内容 → 子树”保留链被解除；显式保留 LoaderInfo 的脚本仍能保持其内容，符号晚实例化继续有效。

已完成：核心146 passed/1 ignored；Loader回归52 passed/2 ignored；root回归30 passed/1 ignored；SymbolClass 6 passed；大型预加载3 passed。组间有重叠，含既有已知失败的预期校验，不代表91个唯一新测试。普通桌面cargo check、诊断release构建、Python20测试和格式检查通过。

当时的候选EXE SHA256 `399e231a4715db50e25b6db5dba21a5a8ddd1c3968b3aad3e58aada26fcc93f3`，副本保存在 `target/memory-diagnostics/candidate-20261027-221829/ruffle_desktop.exe`；release根目录已更新为最新候选。两轮旧实机数据均保存独立EXE/PDB副本，分析仍对应修复前程序。**尚未实机量化节省，也未宣称解释全部5GB。** 已完成首轮候选实机游玩，末段原地内存仍增长，数值日志已保全；结果和限制见 [CANDIDATE_TEST_ANALYSIS.md](CANDIDATE_TEST_ANALYSIS.md)。末段 SWF/类表/网格基本稳定，GC 反复降回约400万对象，同时 ScriptObjectData 每秒创建约5.07万个。不能据此宣称整体问题解决或量化节省。

本次同时保留 `Start-GameHeapCapture.ps1`（F8/F9/自动冷却/F10，完成弹窗）和大文件流式 `compare_heap.py`；独立 Rust v0/legacy 符号解码工具在 `heap-symbol-demangler/`，不影响播放器依赖。构建及调用见 README。下一步优先评审实例化/LoaderInfo兼容语义和此候选对实际对象树保留的影响；Vulkan驱动及类/影片定义缓存仍是独立待查项。

## 修复前的晚间采集与隔离复现记录


已实现并实跑 `Start-GameHeapCapture.ps1`（F8基线、F9停战后自动120秒快照、F10提前结束；-NotifyOnFinish在保存后弹窗），以及流式 `compare_heap.py`。第一次游戏退出较早，留下两张有效快照；第二次完整三阶段已成功，无需再安排相同实机测试。

完整采集目录 `target/game-heap-capture/20261027-210135-954`：Windows heap 1164.09→2549.39→2413.30MiB，冷却后相对基线净增1249.21MiB。private commit 2406.24→6820.29→7057.75MiB；working set 1905.61→4232.99→4467.10MiB。指标不可相加，整体接近5GB尚未完整解释。保留来源涉及Vulkan驱动、事件、显示对象和脚本/类表；GC确实完成周期但地图仍有新分配，不能将所有保留视作泄漏。数字和限制见该目录analysis-zh.md。

新增离线测试 `player::loader_root_retention_tests::registered_root_prototype_retains_loader_content_and_its_children` 已验证：根SymbolClass原型共享LoaderInfo，形成 Library原型→MovieClipShared.loader_info→LoaderInfo.loaded_stream→原加载根内容→子树 的保留链。无原型则GC可回收；有原型时保持存活；仅清除LoaderInfo.content引用即可使根及子树回收。原型克隆早于子树创建，且观察用GcWeak。此为隔离的保留机制复现，不等于实机全部5GB根因已定；生产生命周期尚未修改。下一步优先围绕此复现验证修复及晚实例化/LoaderInfo兼容语义，停止反复手动测试。

以上为修复前记录。旧复现测试已在修复提交中改为 root_symbol_does_not_retain_loaded_tree_but_live_loader_info_does；工具和摘要同步保存。下面白天交接中旧测试数量已过时，以顶部状态为准。

## 白天交接时的结论

**尚未定位解释游戏几 GB 占用的具体分配源，也没有完成根因修复。** 已有两项实证：

1. 诊断构建中的可选闲置缓存回收可以降低网格和上传位图的保留量；进程私有提交仍增长，GPU 分配器 reserved 也可能保持高位。这是局部候选，不是整体验证通过的内存修复。
2. Windows 堆快照 → 本地 PDB → xperf 离线导出 → 具体 Rust 分配函数归属的流程已用真实离线样例跑通，可以进入有明确目标的游戏分配栈采集准备。

用户已多次长时间手动游玩，明确表示测试负担过大。**不要再按历史文档要求反复游玩 20 分钟来观察总内存。** 白天另一台电脑推进源码、工具和离线验证；本机 Windows 的游戏实测留待用户晚上可操作时。

## 已保存的实现

- `MemorySession.cs`：observe 模式切出/最小化继续采样，标记 background；返回前台继续，F10 可在后台结束。录制/回放模式保留原焦点约束。
- `summarize.py`：后台样本不进入前台表格；出现后台间隔的阶段不报告跨间隔增长斜率；显示缓存策略变体。
- `Start-DiagnosticSession.ps1`：独立诊断 EXE、日志目录与采样 PID；验证构建特性、记录哈希及策略，不依赖本机缺失的 Get-FileHash。
- `summarize_resources.py`：只提取允许的数值指标，与 OS 样本时间对齐；区分瞬时量、累计量和逐次 sweep 活动，不把资源估计与进程/GPU 内存相加。
- 诊断特性新增动态属性 HashTable 分配布局与 AVM2 Vector 容量统计，覆盖预留、清空后容量保留和克隆容量重算；不新增 GC 根。
- `RUFFLE_DIAGNOSTIC_SWF_CACHE_IDLE_SECONDS`：仅 `performance_diagnostics` 构建读取。未设置/0 默认关闭，90 是本轮测试值；只回收可重建渲染缓存，不卸载 SWF 定义或脚本对象。普通桌面构建不启用此环境开关。
- 缓存 sweep 增加诊断耗时与活动计数；离线测试覆盖共享网格、待提交绘制句柄、导出符号晚实例化和 Morph 重建。
- `heap_fixture.rs` / `Test-HeapAttribution.ps1`：独立样例产生 0 → 18 MiB → 2 MiB → 0 的已知 payload；管理员分支自动对该样例 PID 拍三次堆快照，使用独立命名 WPR 会话，不改持久 IFEO。
- `Export-HeapSnapshots.ps1` / `summarize_heap.py`：xperf + 本地符号离线导出；验证快照数、分配数和大小之和，忽略末尾全局栈目录，输出每快照 JSON；支持严格校验离线样例。

## 证据摘要（数字可随 Git 带到另一台电脑）

### 卡布西游实机

以下 session ID 均为本机文件夹名。此处 MiB 为 1,048,576 字节，主指标是进程 private commit；不可与工作集和 GPU 列相加。

| 数据 | 结果 |
|---|---|
| 普通构建 `memory-observe-20261026-214541-948` | 连续对战约 21.68 分钟，阶段首尾 private commit 约 2587.0 → 5120.7 MiB；冷却结束约 5156.0 MiB。 |
| 诊断基线 `resource-diagnostic-20261026-224430-786` | 对战约 21.55 分钟，2813.2 → 4726.5 MiB；冷却结束约 4768.2 MiB。SWF、角色定义、网格和位图保留增加，但现有指标不能解释完整进程占用。 |
| 缓存候选 `resource-diagnostic-idle-render-cache-candidate-20261027-000150-146` | 开启 90 秒回收；前台约 15 分钟区间 private commit 3178.3 → 4033.6 MiB；末尾约三分钟切出后结束为 4158.1 MiB。用户确认末段已停止对战、没有明显卡顿；没有最终 F9 冷却标记，不能把整段阶段 2 都当作战斗。 |

诊断基线/缓存候选的对战相对时间 14.5–15 分钟、前台约 30 秒窗口：

| 指标 | 基线 | 缓存候选 |
|---|---:|---:|
| private commit MiB | 4175.4 | 4033.6 |
| 网格顶点/索引 MiB | 233.1 | 11.0 |
| 上传位图像素估计 MiB | 202.1 | 21.2 |
| GPU allocator allocated MiB | 688.60 | 277.66 |
| GPU allocator reserved MiB | 1536 | 1536 |

**输入和游戏负载没有严格配对，不能据此声称精确节省百分比。** 候选内部能看到 SWF 增加时缓存量回落，支持回收生效；仍不能解释全部 private commit 增长。早期 focus_lost 和掉线中断轮次不能作为完整效果对照。候选日志过滤器原漏掉 library sweep，现已补 `ruffle_core::library=info`；不得补造缺失的历史释放次数。

### 离线堆归属：真实 ETL 已通过

本机目录：`target/heap-attribution/20261027-010929-552`，PID 26252。

| 快照 | 分配数 | 实际堆字节 | retained 函数 payload | transient 函数 payload |
|---|---:|---:|---:|---:|
| 分配前 | 14 | 7,025 | 0 | 0 |
| 分配后 | 88 | 18,883,121 | 2 MiB（8 块） | 16 MiB（64 块） |
| 释放后 | 23 | 2,104,369 | 2 MiB（8 块） | 0 |

每块 256 KiB，函数 `ruffle_heap_fixture::retained_allocation_site` 和 `transient_allocation_site` 均从真实分配栈解析成功。丢失事件/缓冲区均为 0。验证到函数名，未验证源码行号；系统 DLL 仍可显示地址。堆总量还包括 Vec 元数据及运行时，不能要求等于样例 payload 或进程 commit。

### 引用链与统计限制

- 已用离线测试证明 `Library → characters → shared definition → Arc<SwfMovie>` 可使弱 movie key 继续存活；移除整个 library 根后才释放。这只是保留机制证据，尚不能证明实机对应电影本该卸载，也不能当作全部问题根因。
- MovieLibrary 的应用域、类定义、SymbolClass、TranslationUnit 及晚实例化语义相互关联。不要凭 Arc 强引用数、直接弱化域/定义，或跳过 GC tracing 来“强制释放”。
- **Cargo.lock 实际使用 registry gc-arena 0.7.0。** `gc_debt` 是基于对象数量的 pacing 工作量，不是字节。它没有 GC 堆字节接口。曾参考本机另一个 checkout 得出字节解释，现已纠正代码、报告及文档。
- Array 稠密/稀疏存储和其他原生分配仍存在统计盲区。Windows heap 快照不代表所有直接 VirtualAlloc 或 GPU 分配；需与 private commit、GPU allocated/reserved 对照。

## 另一台电脑白天可以继续的任务

按以下顺序推进，无需用户启动游戏：

1. 以现有 `heap-summary.json` 结构实现通用的快照差分/按应用调用栈聚合与排序；按栈帧归组，不能依赖 StackID/地址在不同快照间保持稳定。先用合成输入和离线 fixture 验证新增、持续保留及释放的识别。
2. 准备面向明确游戏 PID 的短时堆快照采集脚本，复用已验证的命名 WPR session、PID 配置、finally 清理和本地 EXE/PDB 配对。该游戏入口现已实现并完成三快照采集；不要重复开发或重复安排已完成的基线轮次。
3. 继续离线检查 SWF/类/域保留链或补 Array 容量统计；保持渲染缓存候选默认关闭，避免在没有语义证明时改变对象生命周期。

晚上本机可用时，再进行一次有明确问题的短采集：在关注的分配发生前启用 PID 跟踪，取操作前、操作后和停止操作后的快照，按仍持有字节的调用栈找来源。不要以“再玩久一点看总量”作为采集目标。尚未证明几分钟内一定复现长时间增长；采集边界应以具体操作和新分配证据制定。

## 已通过的验证

- `cargo test -p ruffle_core --lib --release --locked --features performance_diagnostics --target-dir target/memory-diagnostics`：145 passed、0 failed、1 ignored（既有手工实验）。
- `cargo check --locked -p ruffle_desktop`：成功。
- `cargo build -p ruffle_desktop --release --locked --features performance_diagnostics --target-dir target/memory-diagnostics`：成功。普通/诊断构建有既有 UI dead-code 和 Git build metadata 警告。
- Python memory-session `test_*.py`：15 passed。
- `cargo fmt --all -- --check`、`git diff --check`：通过。
- Windows PowerShell 5.1 fixture ValidateOnly、隐藏进程四阶段协议及真实管理员 WPR 三快照均完成；xperf 导出脚本和 fixture 自动核验实跑通过。

新诊断 EXE SHA256：`a4464fff47df41c2dd779b2e07b8e45b2ead9db79aafb4dfe1b1b4180f6ab4be`。不随 Git 传送，另一台电脑需重建。

## 取回代码和验证入口

```text
git fetch private
git switch codex/roco-memory-investigation
git pull --ff-only private codex/roco-memory-investigation
python -m unittest discover -s desktop/tools/memory-session -p "test_*.py"
```

本地没有分支时使用 `git switch --track private/codex/roco-memory-investigation`。没有该远端时先执行 `git remote add private https://github.com/aojiaoxiaolinlin/ruffle-A.git`；远端名可不同，以指向私有 `aojiaoxiaolinlin/ruffle-A` 的远端为准。保留另一台电脑既有未提交改动。

Windows 本机重做离线验证（只有实际录制 ETW 的步骤需要管理员）：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\desktop\tools\memory-session\Test-HeapAttribution.ps1 -ValidateOnly
# 管理员 PowerShell 去掉 -ValidateOnly，得到 <session> 后：
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\desktop\tools\memory-session\Export-HeapSnapshots.ps1 -SessionDirectory "<session>"
python .\desktop\tools\memory-session\summarize_heap.py "<session>" --verify-fixture
```

## 数据留存边界

Git 带走源码、测试、工具与本文中的结果摘要。`desktop/tools/memory-session/sessions/` 和 `target/` 被忽略，包含的 CSV、原始日志、ETL、EXE/PDB、symcache 只在本机；换电脑后路径不存在属正常。原始游戏日志可能包含票据或登录信息，不加入 Git。本文的数值摘要足以继续工具开发；若以后确需原始数据，应另行传送必要且经检查的资料。

`WINDOWS_HANDOFF.md` 保留时间顺序和历史测试细节，其中原有“首轮 20–25 分钟”步骤已完成，当前以本文为准。
