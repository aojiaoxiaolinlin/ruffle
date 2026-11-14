# 0.7.1-A 性能与生命周期整理合并

2026-11-14，Asia/Shanghai。按用户“v16 也合并吧”的授权，本批包含 v16 的时间轴脚本子树剪枝，以及已验证的基础生命周期修复和独立缓存/渲染策略。这里记录合并代码的范围与验证边界，不宣称完整游戏后期掉帧或持续内存增长已解决。

## 合入内容

- LoaderInfo 由实际加载实例持有，符号原型不保活旧加载树；内部显示列表事件在没有可能监听者时减少临时分配。
- 类符号全局索引使用受 GC 正确追踪的弱引用。实际实例、AVM1 动作/函数、字体、活动声音、跨影片符号和在途 Loader 保持必要的定义强持有关系，避免单独改弱索引破坏仍在使用的内容。
- 暂停状态不沿用已到期的帧/定时器期限反复零等待唤醒；恢复及单步路径保留。
- v16 使用现有标志位保存保守、只增不减的脚本子树摘要。注册 callable、待执行脚本、换父节点时传播；AVM2 按钮保持可执行资格，影片重载保留标志。只跳过确定不需要运行帧脚本的子树，enter/construct、绘制、广播及 goto 顺序保留。
- 可选 SWF 图形/不可变位图填充缓存到期策略、独立嵌入 SWF 字形缓存策略及独立 FontFace 绘制缓存策略。字形索引弱持有、每次工作有界，保留轮廓/度量和再次渲染/精确命中的恢复路径。
- 可选 2D 渲染提交预算，在完整 Draw/Blend 块结束处提交，保留目标分支的 Surface、Globals、copy 和 Stage3D 上传实现。无效预算配置关闭实验提交，不中断播放。

生产来源包括 `36bfdd56c`、`aecd81a51`、`7110b44c9`、`94c81657b`、`d0eac7411`，以及调查树中经隔离验证的暂停期限、v16 标志、缓存和完整块提交逻辑。目标基线为 `707be95af598145b7bf0c8227d18207d7a6348dd`；保留 `fe8dc00a6` 的异步 ImportAssets/loadBytes 续预加载、`707be95af` 的初帧脚本前延迟 Loader 事件、`0c0fa8be6` 的 Adreno/GLES 修复。Windows 输入繁忙帧调度和紧凑 VTable 已在基线内，不重复搬运。

## 默认与平台

基础生命周期修复、暂停期限修复与 v16 脚本子树剪枝随正常构建启用。下面的独立策略不默认开启，合并代码不改变其验证门槛。

| Cargo feature | Desktop 环境入口 | 默认与限制 |
| --- | --- | --- |
| `memory_render_cache` | `RUFFLE_SWF_CACHE_IDLE_SECONDS`；`RUFFLE_SHAPE_CACHE_IDLE_SECONDS` 可覆盖为仅图形 | Feature 默认关，TTL 默认 None；0 关闭到期策略；合法秒数 1–86400 |
| `memory_swf_glyph_cache` | `RUFFLE_SWF_GLYPH_CACHE_IDLE_SECONDS` | Feature 默认关，TTL 默认 None；独立于图形和 FontFace；合法秒数同上 |
| `memory_font_face_cache` | `RUFFLE_FONT_FACE_CACHE_IDLE_SECONDS` | Feature 默认关，TTL 默认 None；不初始化尚未使用的字形；合法秒数同上 |
| `memory_pass_budget` | `RUFFLE_RENDER_CHUNK_PASS_BUDGET` | Feature 默认关，预算默认 0；支持 0/32/128；块提交只用于 Vulkan |
| `vtable_memory_experiment`（基线已有） | Cargo feature | 仍默认关；Android 验证门槛见 `AVM2_VTABLE_MEMORY.md` |

完整块提交在其他后端、开启 timer/debug profiler、`render_debug_labels` 或 Tracy 的路径上回退原提交行为。Stage3D 临时绘制 encoder 仅适配共用类型，不使用块轮换。该策略不表示 GLES 已有相同收益。Android/GLES 设备运行和冷恢复卡顿尚未验证，后续设备验收前不提升默认配置。

## 本轮目标代码验证

均从隔离的目标代码与独立构建目录运行；不使用旧 v16 EXE 替代目标验证，不操作用户游戏。

| 检查 | 结果 |
| --- | --- |
| Core 默认 / Common | 157 / 10 通过，0 失败 |
| Core 三个缓存 feature 与紧凑 VTable / Common | 180 / 10 通过，0 失败 |
| AVM2 输出回归（含参考及原创用例） | 1181 通过，0 失败，121 忽略 |
| AVM1 输出回归 | 771 通过，0 失败，19 忽略 |
| 原创仓库顺序用例单独重跑 | 2 通过，0 失败；包含加载、换父节点、卸载与脚本变更，仅检查端点不变量 |
| Wgpu 默认单元测试 | 13 通过，0 失败，2 忽略 |
| Wgpu 提交 feature 单元测试（最终版本） | 14 通过，0 失败，3 忽略；包含无效预算配置不会中断播放的检查 |
| 三个缓存 feature 各自独立构建 | 三组 core cargo check 通过 |
| Desktop 默认及缓存/提交/VTable 启用构建 | 两组 cargo check 通过 |
| Wgpu 提交 feature 与 render_debug_labels | cargo check 通过 |
| 完整像素矩阵 | 每后端 48 场：4 种 RGBA/BGRA/linear/sRGB backing × 2 种质量 × 6 种混合、掩码、滤镜和缓存组合；两臂完整 RGBA 相同 |

像素矩阵使用同一冻结测试程序的独立进程。Vulkan 预算 128 实际执行 120 次块提交；DX12 两臂块提交均为 0。矩阵测试在默认单测中显式忽略，本轮已手动运行两后端对照。其他忽略项保留原状态，不能列作通过。这些检查证明所测范围的兼容性，不是实机 FPS 或长时间内存收益测量。

首次回归工具运行因工作目录错误产生 exit 101/零测试；改为项目 tests 目录后有效套件全部通过。另修正了构建产物 JSON 的 bin/test 选择；旧失败记录保留，不视为引擎回归成功或失败。

## 暂留范围与后续

本批未迁移整套调查 GC、逐对象/原生堆/GPU 诊断、采样启动器、游戏盒登录和 UI，以及 Graphic 原始 Shape 释放、BitmapCache 描述存储、深度/矩形内联、ABC/metadata 容量和 Windows 空闲堆候选。独立候选需另行验收和迁移，不能把本次基础合并称为旧 v16 发行包全部功能的复制。

仓库背景异常记录为原因未定、后续多次未复现；按用户意见暂停追查。原创顺序测试通过不表示复现或修复了真实游戏异常。后期掉帧、持续内存增长以及再次长期游玩的量化验收仍待完成。

本次合并不生成新发行包、不更新旧 v16 程序、不切换调查工作目录，也不执行远端推送。实际本地目标提交与保护工作树的核对收据保存在证据目录。

本轮证据：`D:/Code/Rust/ruffle-memory-evidence/base-merge-review-20261114/` 下的 `verification-v1/result.json`、`chunk-pixels-v1/result.json`、`final-validation/result.json` 和 `MERGE_RECEIPT.json`。较早 `MERGE_READINESS.md` 是授权纳入 v16 之前的预检，不代表最终合并状态。
