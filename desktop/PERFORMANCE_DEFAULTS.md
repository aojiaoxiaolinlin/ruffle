# 基础播放器与游戏盒默认优化

按用户确认，渲染缓存回收、SWF 字形缓存回收、FontFace 绘制缓存回收、
紧凑 VTable 和 Vulkan 提交预算成为正常代码路径。五个旧实验 feature 已删除，
core、desktop 和 renderer 无需额外 `--features`，`--no-default-features`
也不会移除这些已定型优化。诊断和未定型实验保留各自 feature。

core 的三个缓存独立使用 90 秒闲置期限，保留在用保护、有界处理、轮廓/度量
及按需恢复。普通 desktop 启动沿用这三个默认值；其它 frontend 不设置环境
变量也具有 core 默认策略。紧凑 VTable 固定使用单个命名空间/属性内联容量，
额外命名空间仍通过 SmallVec 扩展。

完整 2D 块提交预算默认 128，只在运行时实际 backend 为 Vulkan 时生效。
GL/GLES、DX12、Metal、BrowserWebGpu 保留原提交路径。Android 使用 Vulkan
时具备相同后端条件，但本轮没有做 Android/GLES 设备验证，不宣称移动端收益。
Tracy、debug labels 或激活 timer/debug profiler 的路径继续保留原 encoder。

内部环境覆盖用于故障定位和临时回退：缓存期限变量的 0 关闭对应到期策略；
`RUFFLE_RENDER_CHUNK_PASS_BUDGET=0` 关闭块提交预算。普通用户无需设置变量，
编译者无需配置 feature。Graphic 原始轮廓缓存和 Windows 空闲堆回收未纳入。

本次先进入 `0.7.1-A`，再合到 `aojiao-game-box`。仅游戏盒构建发布 EXE，
日志名与低日志设置沿用游戏盒分支。本次构建身份必须记录固定默认策略、
实际 feature graph 和无环境覆盖的运行验证，不能复用先前默认关闭的发布身份。

基础代码验证：core/common 181/10 通过；renderer 16 通过、3 忽略，其中像素矩阵
已另行手动执行。Vulkan、DX12 各 48 场的默认/关闭两臂完整 RGBA 一致，默认
Vulkan 实际执行预算提交、DX12 为零。desktop 54 通过、1 忽略；AVM2 1181 通过、
121 忽略，AVM1 771 通过、19 忽略。诊断 debug-labels 构建及 core
`--no-default-features` 构建通过。首轮三项旧“默认关闭”测试失败记录保留，
已改为显式关闭以继续检查同样的保护边界；重跑全部通过。
