# 无监听器的显示列表事件：减少临时分配

## 依据与范围

候选首轮实机日志显示末段 ScriptObjectData 约每秒创建 5.07 万个，GC 仍能反复回收。修复前堆快照中，较大的 EventObject::bare_event 分配栈经过 fire_added_events、dispatch_added_event_only 和 dispatch_removed_event，提供了一个具体调查入口。这些旧快照不能证明本轮的全部周转量都来自同一路径。

源码确认：added、removed、addedToStage、removedFromStage 在目标和祖先都没有监听器时，仍创建 EventObject 和事件名字符串，然后走完整派发流程。这些内部事件没有脚本构造器或可观察的无监听回调。优化仅在没有可能的接收者时跳过构造；不是保留根修复，不改 GC 或对象卸载策略。

## 实现与兼容边界

events::has_event_listener_in_chain 只读取已有 dispatch list，不创建列表或调用脚本。检查目标与整条父显示对象链，跨过尚未构造 AVM2 对象的中间父级，匹配 dispatch_event 的遍历方式。

检查保守地包含祖先上的所有监听器。特别是非冒泡的 addedToStage/removedFromStage 仍可能被祖先捕获，不能仅判断目标是否有监听器。有任何监听器时继续使用原来的 EventObject 构造和派发流程。递归派发仍逐节点即时检查，不因父节点没有监听器而跳过后代；前一个回调添加的监听器也不会被预先缓存的结果遮蔽。

四个事件名加入 common strings，避免为了判断是否有监听器而创建临时字符串。只在四个内部显示列表事件入口使用探测，不修改脚本主动 dispatchEvent 的行为。

## 离线验证

- 新分配回归：1000 轮、每轮四种事件、无监听器，修复前增加 **8000 个 GC 对象**（25,107→33,107，断言失败）；修复后增加 **0 个**（断言通过）。这是该离线情形的对象数量，不是游戏节省字节数。
- 新兼容回归：祖先监听使用 capture=true，中间父级没有构造 AVM2 对象，四种事件探测均为 true，并用 simulate_event_dispatch 验证确实可到达处理器；移除监听后探测回到 false。
- 核心 release + performance_diagnostics：148 passed，0 failed，1 个既有手工实验 ignored。
- 既有预编译 SWF 回归过滤 event：115 passed；包含框架既有已知差异的预期校验，不能当作115项无已知问题的 Flash 一致性验证。
- 既有 simplebutton_added_to_stage：1 passed。
- 普通桌面 cargo check：通过。

测试使用仓库已有 SWF/参考输出，RUFFLE_TEST_OPTS='--compile-mode use-precompiled'。构建中的 Git 元信息、既有 dead-code 和增量缓存硬链接警告不影响上述退出状态。

尚未实机量化该优化效果，不能声称每秒5.07万个分配已消失，也不能将原地内存增长全部归因于这些事件。新诊断构建同时包含先前的 LoaderInfo 修复，下一次实测须区分这两个构建；首轮候选的原 EXE/PDB 已和日志保存在 target/memory-diagnostics/candidate-20261027-221829/。

## 可运行成品

路径：`target/memory-candidates/event-listeners-20261027/ruffle_desktop.exe`，SHA256 `87a606d6382a31fa85f7f562acd8bd1bff3c86fcac03963b6fc387e806149149`。同目录PDB和build-identity.json已保留。已解析EXE CodeView及PDB info stream，GUID/age一致；--version成功退出。

release诊断构建完成编译、链接后，Cargo向旧release根目录复制EXE时失败，因为用户22:58:15启动的旧EXE仍运行。未终止游戏；改为将本次release/deps下已经链接的EXE/PDB复制到上述独立目录。构建命令本身的最终退出码不是成功，不应把这个最后复制失败隐藏为整条命令通过。
