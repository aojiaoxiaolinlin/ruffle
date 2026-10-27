# LoaderInfo 持有旧根内容：修复候选

## 问题与最终实现

根 SymbolClass 注册会通过 `MovieClip::instantiate` 克隆根影片，保存为库中的符号原型。原先 `LoaderInfo` 属于 `MovieClipShared`：原型因此通过 `LoaderInfo.loaded_stream` 指回原来的加载根，并保留它在克隆之后创建的整棵子树。即使根内容不在舞台上、外部也不再持有它，库仍能成为该子树的 GC 根。

现在将加载信息移到 `MovieClipData` 的实例字段。实际播放器根/Loader 内容继续持有自己的 LoaderInfo，事件和访问器使用该实例字段。实例化可复用符号时清除克隆对象的加载上下文，仍共享原来的影片定义、预加载进度和类绑定。因此库原型不再通过 LoaderInfo 持有旧内容，后续符号实例化也不会重新继承旧加载的内容引用。

修复没有强制 GC、跳过 tracing 或按 Arc 引用数删除对象。显式持有 LoaderInfo 的脚本仍可保留其内容；正常 unload 的流重置行为不变。

## 对照与兼容验证

先前的隔离复现验证了旧路径会保留根和子树，仅清除 LoaderInfo.content 便可回收。应用本修复后，该旧行为断言如预期失败（原来应存活的对象已被回收），随后将其改为真正的回归契约：

`player::loader_root_retention_tests::root_symbol_does_not_retain_loaded_tree_but_live_loader_info_does`

- 不注册原型，根/子树可以回收。
- 注册根原型，根/子树仍可以回收。原型早于子树创建，排除原型容器直接克隆子树的影响。
- 外部显式保留 LoaderInfo 时，根/子树继续存活；仅重置流后可回收。
- 实际加载根的 LoaderInfo 身份保持不变，原型不继承它。
- 原内容回收后，库定义仍存活，可以再次实例化符号。

验证结果（2026-10-27）：

| 检查 | 结果 |
|---|---|
| 核心 release + performance_diagnostics | 146 passed，1 个既有手工实验 ignored |
| SWF 回归 `loader` | 52 passed，2 ignored |
| SWF 回归 `root`（含 instantiate_root_character） | 30 passed，1 ignored |
| SWF 回归 `symbolclass` | 6 passed |
| SWF 回归 `large_preload` | 3 passed |
| 普通 desktop cargo check | 通过 |
| 诊断 desktop release build | 通过 |
| 内存工具 Python 测试 | 20 passed |

SWF 组使用仓库已有预编译 SWF/参考输出，组间有重叠，部分测试含已标注已知失败的预期校验；这些数量不代表新增的唯一 Flash 一致性用例数。运行时未出现新的回归失败。未启动新的手动游戏测试。

## 结论范围

这是一条已离线证明、现已解除的保留链，尚不能宣称它解释洛克的全部约 5 GB，或已实测游戏节省了多少。两轮实机快照都使用修复前的 EXE/PDB，副本已留在各自采集目录，后续重建诊断程序不会改变它们。

完整修复前洛克数据：`target/game-heap-capture/20261027-210135-954`。Windows heap 为 1164.09 → 2549.39 → 2413.30 MiB；最后两分钟冷却后相对基线多 1249.21 MiB。较大分配来源还包括 Vulkan/NVIDIA 驱动、事件对象和类表；它们需要各自归属，不能把本修复视为全部进程内存的解释。地图仍有持续分配，快照还可能含未 sweep 的垃圾。

下一步应先审阅和扩展必要的离线生命周期兼容测试；实机验证以这个明确候选为对象，再检查旧内容是否减少保留。不要再安排没有候选的长时间纯总量观察。
