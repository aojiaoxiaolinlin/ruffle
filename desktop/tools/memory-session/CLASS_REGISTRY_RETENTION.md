# 类注册表保留定义：隔离复现与候选修复

日期：2026-10-28，Asia/Shanghai。接续事件优化版的卡布实机数据。

## 当前结论

已离线证明并解除一条独立保留路径：全局 Avm2ClassRegistry 原先强 trace 类定义键；类的初始化 Method 指向 TranslationUnit，后者强持有 SwfMovie。即使不注册任何影片库、字符原型或显示内容树，注册表仍能独自保活定义影片。

本候选将注册表键改为正确 trace 的弱 GC 身份，保留指向影片的弱 Arc。仍有外部类引用时，查找及首次 SymbolClass 绑定优先规则不变。不是跳过 GC tracing，也没有按 Arc 强引用数量强删资源。

**影片库的 application domain 是另一条仍存在的强根。** 所以本候选不能保证游戏中的 SWF 数量减少；如果每份定义仍被影片库域或字符原型持有，去掉注册表根也不会使其回收。尚未实机量化，未声称解决全部4–5GB。必须保留仍可访问的域/类和晚实例化语义，不能直接清空这些根作为产品修复。

## 卡布长测：版本与现象

PID8636，2026-10-27 23:35:33 启动，路径 target/memory-candidates/event-listeners-20261027/ruffle_desktop.exe；文件哈希匹配已发布事件候选 `87a606d6382a31fa85f7f562acd8bd1bff3c86fcac03963b6fc387e806149149`。用户报告长时间对战约4GB，停止操作后仍有轻微增长。只读检查时进程 working set 4,940,578,816 字节（约4711.70MiB），private commit 8,255,598,592字节（约7873.15MiB），与任务管理器显示列口径可能不同，不能相加。

最终日志保存于忽略目录 target/memory-diagnostics/kabu-event-candidate-20261027-233533/ruffle-final.log，7,107,612字节，SHA256 `ebec7931e973e6279bba34070dd712d0cf75052882780bc5234b7a544d258100`。覆盖本地23:35:35至次日00:07:42。该进程随后已退出。resource-final-numbers.json 是白名单数值提取；原始日志不提交，可能含游戏连接数据。

23:50→次日00:00各分钟中位数：已知SWF 1516→2251；character定义84651→132716；SWF payload 175.7→263.1MiB；网格顶点/索引488.05→771.8MiB；GPU allocated 1178.25→1620.46MiB。资源统计可能重叠，不相加，也不是严格配对的前后快照。

00:00至最终日志：SWF先从2241增加到2251，之后稳定；网格764.6→777.1MiB；GPU allocated 在1422.92–1799.13MiB间波动，首末1619.99→1614.88MiB，reserved固定3072MiB。GC对象在约250.6万–378.3万间波动。没有精确停战热键，不能把00:00视作严格停战边界；这些记录不能将用户观察到的最后轻微OS内存增长归属到单个函数。

## XML／对象优化历史核对

desktop/PERFORMANCE_DIAGNOSTICS.md 记录2026-10-25带XML查询计数的基线发生点击无效，而该故障构建 compact=false；旧基线恢复正常，离线鼠标探针也能触发事件。当时未证明故障来自XML容量压缩或计数本身。

本次卡布所用事件候选的Cargo构建fingerprint只开启performance_diagnostics及普通桌面功能，未启用xml_query_diagnostics、xml_query_memory_experiment、vtable_memory_experiment。因此不能把本次增长归因于这些实验逻辑已启用。常规XML对象仍可能贡献分配，现有日志没有XML类型级别归属，未排除其普通占用。

## 实现与离线验证

Class::weak_identity 返回 GcWeak<()>，只用作弱身份，不通过擦除指针读取Class数据。注册表由usize地址键及含弱身份、弱影片和symbol ID的值组成；derive Collect正确标记弱边。弱身份使分配地址不会在旧条目仍存在时被复用。清理死条目按阈值摊销，避免逐次注册扫描造成二次复杂度；不主动强制GC或扫描玩家对象。

两个新增生命周期测试：

1. 无注册根／仅注册表／仅影片库域／两者都有。使用最小未执行ABC方法形成实际 Class→Method→TranslationUnit→movie 关系，并在局部域中导出类，构成可由GC正常处理的内部环。修复前，仅注册表或仅库域都能独立保活；修复后，仅注册表不再保活，库域仍保活。只在隔离测试中移除所有库根后可回收。没有显示树或字符原型混淆因素。
2. 显式动态根持有Class时，经过多个完整GC周期，符号查找仍指向原影片；同影片不同ID、不同影片重绑均不覆盖首次绑定。释放动态根后，影片可回收，即使注册表仍在。

验证结果：核心release+performance_diagnostics **150 passed / 1 ignored**；预编译SWF Loader组52 passed/2 ignored，SymbolClass 6 passed，root 30 passed/1 ignored，application_domain 1 passed。组间可能重叠，含框架既有已知差异的预期校验，不代表新增89项Flash一致性用例。普通desktop cargo check和诊断release完整构建通过，格式和diff检查通过。

## 可运行成品与限制

target/memory-candidates/weak-class-registry-20261028/ruffle_desktop.exe，SHA256 `ccb073355f697dcb4abc46567e3113f0e046d39b385642e2b0d58deeab9cae3c`，同目录保存PDB及build-identity.json。该构建包含此前LoaderInfo和事件分配修改；三个XML/VTable实验仍关闭。--version正常退出。没有启动新游戏或要求用户重复长测。

下一步仍是影片库的域/字符定义所有权：需要区分仍被脚本合法持有的定义与仅被库自身反向保活的定义。这里涉及混合Arc与GC生命周期，不能用跳过trace、Arc引用数量猜测、清空域、按停战计时卸载定义等方式替代可达性设计。
