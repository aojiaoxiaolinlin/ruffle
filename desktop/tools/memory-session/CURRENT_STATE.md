# 内存排查当前状态与跨电脑交接

更新时间：2026-10-27（Asia/Shanghai）。工作分支：`codex/roco-memory-investigation`；仓库：`aojiaoxiaolinlin/ruffle`（本机远端名 `fork`）。

## 当前结论

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
2. 准备面向明确游戏 PID 的短时堆快照采集脚本，复用已验证的命名 WPR session、PID 配置、finally 清理和本地 EXE/PDB 配对。当前只有 fixture 采集入口，**游戏专用入口还未实现**。
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
git fetch fork
git switch codex/roco-memory-investigation
git pull --ff-only fork codex/roco-memory-investigation
python -m unittest discover -s desktop/tools/memory-session -p "test_*.py"
```

本地没有分支时使用 `git switch --track fork/codex/roco-memory-investigation`。远端名可不同，以指向 `aojiaoxiaolinlin/ruffle` 的远端为准。保留另一台电脑既有未提交改动。

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
