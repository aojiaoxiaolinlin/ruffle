# 局部混合目标与柔和滤镜分辨率

开发基准：`0.7.1-A`，起点 `297deadf9`。开发分支：
`codex/local-blend-filter-resolution`。安卓真机验证完成前不合入基准。

## 复杂混合的局部目标

Multiply、Lighten、Darken、Difference、Invert、Overlay、HardLight 在前景
仅包含普通二维矩形、位图或形状时，使用实际绘制范围创建前景目标，并只复制对应
背景区域。合成使用局部 viewport，保留原像素分辨率。

形状范围来自本次注册的 tessellation 顶点，包含笔画几何；位图范围使用绘制区域及
pixel snapping 后的矩阵。范围保留 2 像素保护边，并向外对齐到 32 像素网格，减少
动画产生大量不同尺寸的纹理池条目。与画布相交后，面积达到画布的 75% 时走原路径。

当前保守回退：嵌套混合、遮罩、Alpha/Erase、Shader、Stage3D、线条命令、透视或
非零 Z。全屏、空和完全离屏的范围也保留原路径。内部的独立混合仍可分别优化。
后续可扩大安全覆盖范围；此版不缓存动态显示对象的子树边界。

设置 `RUFFLE_GPU_FULL_BLEND_TARGETS=1` 可在启动时恢复全屏目标，用于 A/B。
该开关在进程内只读取一次。

## 发光 / 阴影的模糊分支

默认 100%，保持兼容。原图与最终合成保留原分辨率，仅 Glow / DropShadow 的
模糊分支允许降到 75% / 50%。独立 BlurFilter、Bevel 及其他滤镜不变。
`BitmapData.applyFilter` 始终保留原精度，不受显示滤镜设置影响。

前端在首次渲染前调用：

```rust
assert!(wgpu_renderer.set_soft_filter_scale(50));
```

有效值为 `100`、`75`、`50`；无效值返回 false 且不改变设置。启动环境变量
`RUFFLE_GPU_SOFT_FILTER_SCALE=50` 也可以配置，缺省或无效值采用 100。
桌面端可直接使用启动参数，命令行优先于环境变量：

```powershell
.\ruffle_desktop.exe --soft-filter-scale 50 "D:\Downloads\GreenGame.swf"
```

`--soft-filter-scale 100` 为原精度，`75` 为折中，`50` 为半宽半高。
不传参数时采用环境变量，再回退到 100。参数只控制发光和阴影的模糊分支，
与画布缩放、`--quality` 和局部混合目标优化分别独立。
通过桌面菜单重开或打开其他文件时也保留该启动选项。
也可在打开对话框的“播放器设置”中，勾选“发光/阴影模糊分辨率”，选择
100%、75% 或 50%，对本次打开的内容生效。命令行指定值会预填到该控件，
可在打开前修改；取消勾选恢复启动环境变量指定值（未指定时为 100%）。
这个提交没有安卓设置页；安卓前端需接入上述公用方法。
运行时修改不会主动使 core 已存在的 CAB 失效，前端若支持热切换需自行使其重建。

模糊前先线性降采样，使用实际舍入后的横纵比例分别缩放核半径，保持迭代次数与
阴影偏移的屏幕尺寸；最后对模糊结果线性放大，与原精度原图合成。原图采样方式不变。
宽或高不足 64、有效模糊轴的宽度不足 8、无实际模糊的情况走原路径。
低分辨率是近似效果，会改变柔和边缘；不保证与 Flash 像素一致。

## 验证（2026-10-20，Windows RTX 4080）

- 18 项混合、64 项滤镜、29 项遮罩、30 项 CAB、2 项九宫格既有回归通过，
  沿用各用例原有容差及预期状态；另有 6 项渲染器单元测试通过。
- Vulkan / 桌面 GLES 的局部混合对照覆盖旋转、越界、透明位图子区域、pixel
  snapping、特殊混合与遮罩回退。回退场景逐像素一致。全 7 种局部复杂混合的
  20 层、8 帧测试中，旋转矩形在第 2 帧有 1 个边缘像素的 MSAA 覆盖率差异，
  最大通道差 30；加入平滑透明位图后最大通道差 38，其余差异主要为 1/255 舍入。
  每帧超过 1/255 的像素最多 1 个，两种后端结果相同。这些局部混合对照采用
  通道容差 1、每帧最多 1 个超差像素；不能宣称所有旋转情况逐像素一致。
- 柔和滤镜探针覆盖 100/75/50%，一像素原图细线保持不变；在 50% 设置下，
  `BitmapData.applyFilter` 的源子区域及目标偏移输出与 100% 逐像素一致。
- 1024×512 发光 + 阴影探针，离屏临时纹理 checkout 峰值：100% 为 6 MiB，
  75% 为 5.125 MiB，50% 为 4.5 MiB。包含仍保留全分辨率的合成目标。
- 桌面 GLES、Medium（2× MSAA）、2800×1280、128 MiB 池预算、20 个局部
  复杂混合层：main checkout 峰值由 95.703 MiB 降到 41.25 MiB；第 12 帧两种
  路径均无新建 scratch。固定探针不代表安卓仓库的实际耗时或帧率。

原始日志、PNG 位于忽略目录 `target/performance-research/local-blend/`。

## 复现命令

```powershell
$env:CARGO_PROFILE_RELEASE_PANIC='unwind'
cargo build --offline --release --locked --target-dir target/performance-build -p ruffle_render_wgpu --examples

# 比较不同柔和滤镜比例；内含原图与 BitmapData 精度断言。
& target/performance-build/release/examples/soft_filters.exe 50 soft50.png

# 原版 / 优化版图片应在相同后端与质量下比较。
$env:RUFFLE_GPU_FULL_BLEND_TARGETS='1'
& target/performance-build/release/examples/scratch_layers.exe 20 640 400 8 before complex-bitmap
Remove-Item Env:RUFFLE_GPU_FULL_BLEND_TARGETS
& target/performance-build/release/examples/scratch_layers.exe 20 640 400 8 after complex-bitmap
& target/performance-build/release/examples/scratch_layers.exe compare before after 8 1 1
```

探针可设置 `RUFFLE_PROBE_GLES=1`，scratch_layers 还可设置
`RUFFLE_PROBE_MEDIUM=1`。其他混合对照模式为 `complex`、`mixed`、`fallback`、
`masks`，后三种回退对照使用严格比较（省略最后两个容差参数）。运行结束清理相关
环境变量。允许极少数边缘像素差异仅用于上述固定局部混合探针，未更改既有回归容差。

## 安卓验收

仍需真机在固定仓库、对战路线比较：先单开局部混合（滤镜 100%），再分别测试
75% / 50%。保持画布分辨率、GLES、Medium、128 MiB scratch 预算一致。
记录 render 的中位数 / P95、scratch 新建字节、checkout 峰值、uploaded / CAB
大小、OOM 与长时间稳定性；截图检查角色、文字、光晕、阴影及透明边缘。

此优化不减少游戏上传图片或持久 CAB 的原图像素，也不处理批量上传导致的加载卡顿。
不改变显存总预算或动态 buffer 策略。

## 新增日志统计

无需额外参数，使用 INFO 日志即可。共享 wgpu 路径在首帧及后续约 239 帧汇总；
原生平台还会在报告间隔达到 5 秒后的下一帧汇总，防止低帧率时长时间没有数据。
每条记录包含实际 `report_frames` / `frames`，应使用它归一化累计量，不要固定除以 239。
调整画布导致重建池时重新开始统计窗口。

- `GPU render settings`：实际 viewport、quality、MSAA 样本数、当前模糊比例、
  局部混合是否启用、scratch 空闲预算及本次累计帧数。默认 100% 也会记录。
- `GPU blend regions`：`main` / `offscreen` 分开；`prepared` 为真正准备的图层，
  延迟命令到实际准备时才计一次；`candidates` 仅包含支持局部目标的 7 种复杂模式；
  `cropped / candidates` 才是候选命中率，原有 `deferred_layers` 不是裁剪命中数。
  `source_area_mpx=原面积->实际面积` 是全部混合源目标累计百万像素，
  `candidate_area_mpx` 是候选子集的面积；包含保护边和尺寸分档，不代表像素覆盖率、
  GPU 字节、同时驻留量或精确节省带宽，也不包含背景副本/resolve/stencil 的全部开销。
- `fallback` 为每个未裁剪图层遇到的首个原因，数量之和等于 `prepared-cropped`。
  `trivial` / `ancestor` / `shader` 是非候选模式；其余原因是候选未裁剪原因：
  `disabled`、`active_mask`、`nested`、`mask_commands`、`3d`、`lines`、
  `invalid_bounds`、`large_bounds`、`empty_offscreen`。不将一次图层的多个条件重复计数。
- `GPU soft blur`：实际评估的发光/阴影模糊次数、真正降低分辨率的次数及源面积变化。
  排除无实际模糊、独立 BlurFilter、Bevel、脚本可读取的 BitmapData.applyFilter 和最终
  全分辨率合成。可判断设置 50% 后是否被小图/小核保护规则大量回退。
- `Desktop perf`：每约 5 秒汇总 tick、player.render、GUI 渲染的调用数、平均耗时、
  P95 上界与最大耗时。单次计时精度为微秒，直方图为 1 ms 桶；P95 落在 >=1 秒
  桶时使用该窗口最大值作为保守上界。直方图固定大小，没有无界逐帧样本或逐帧日志。
  打开/关闭内容时先输出旧窗口，防止混入新设置。tick 是事件循环更新调用，可能一次
  不执行 SWF 帧或追赶多帧；`render_hz` 是 player.render 调用吞吐，不是游戏逻辑 FPS。
  所有耗时是 CPU 墙钟，包含可能的驱动阻塞；不是 GPU timestamp。GUI 项包含窗口
  合成与提交/呈现。统计不主动等待 GPU，也不读回像素。

共同渲染计数也通过 `scratch_texture_stats().0/.1` 的 `blend_regions` / `soft_blur`
返回上一帧快照，安卓前端可以并入自己的 perf 日志；桌面 tick/render 统计仅属于桌面。
不要相加不同窗口的独立峰值，或把累计新建量当成显存占用。
