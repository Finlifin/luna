# 文本排版、双通道字形管线、国际化与光学级渲染质量

> **状态**：`Draft v0.1（讨论稿）`  
> **前置阅读**：[05-layout-scene-and-rendering.md](file:///home/fin/luna/tmp/silver-design/05-layout-scene-and-rendering.md)

---

## 1. 工业级六阶段文字排版流水线

文本子系统是 GUI 中分支最繁杂、CPU 消耗最高的环节。Silver 拒绝“仅支持单行拉丁字符 + 粗暴截断”的玩具排版，直接按国际化工业标准确立确定性的六阶段流水线，并将结果缓存与 Layer 4 的 SoA 布局树深度耦合：

```text
Unicode 字符串流
  │
  ├─► 1. Segmentation      (UAX #29 字素簇 Grapheme Cluster 切分)
  ├─► 2. BiDi              (UAX #9 双向文本嵌套层级：分离逻辑序与视觉序)
  ├─► 3. Itemization       (按 Script、#Locale 语言标签与字体覆盖率切分同质 TextRun)
  ├─► 4. Shaping           (HarfBuzz / 原生塑形 + 单词级 LRU 度量缓存)
  ├─► 5. Line Breaking     (UAX #14 + 中日韩 Kinsoku 避头尾 + 泰语等无空格字典断词 + 可选 Knuth-Plass)
  └─► 6. Positioning       (多基线对齐：Alphabetic / Ideographic + 字符簇映射表 Cluster Map)
```

### 关键工程优化

1. **单词级塑形缓存 + 不可变 `Paragraph` 快照**  
   以“单词 + 样式指纹”为键建立塑形 LRU 缓存；调整窗口大小只重跑第 5–6 阶段折行，不重调 HarfBuzz。
2. **固有宽高边界与 Height-for-Width 查表**  
   一次线性扫描提取段落的 `min_content_width`（最长不可分割词宽）与 `max_content_width`（单行不折行总宽），并缓存不同宽度约束下的高度结果，消除弹性容器多层嵌套测量时的 $O(N^2)$ 退化。
3. **字符簇映射与双向双光标（Split Caret）**  
   维护精确的 `ClusterMap`（处理 `fi` 连字合并与多码点肤色 Emoji），通过二分查找在 $O(\log N)$ 完成像素坐标命中测试与 BiDi 交界处的双光标定位。
4. **编辑器级 Rope + 视口增量排版**  
   针对代码编辑器、日志流与超长文档视图（`TextBuffer`），底层采用 **Rope（绳索树）** 在内部节点聚合换行符计数与行高统计，仅对修改行及当前屏幕视口可见的数十行执行完整六阶段排版（$O(\log N)$ 编辑响应）。

---

## 2. 双通道字形图集与无图集 GPU 曲线混合渲染

为兼顾 95% 常规小字号文本的极高合批密度与超大字号/动态缩放文本的绝对锐利度，Silver 采用双通道分离 + 路径阈值分流：

| 渲染通道 | 格式与量化策略 | 适用对象 | 核心优势 |
|---|---|---|---|
| **单色 Alpha 掩码图集** | **R8** 单通道图集 + **1/4 物理像素**水平亚像素量化 + **笔画增强（Stem Darkening）** | 常规尺寸单色 UI 文本（95% 文本场景） | 显存占用仅为 RGBA 的 1/4；深色背景下自动微增浅色文字笔画权重防止发虚断线 |
| **彩色字形图集** | **RGBA8** 独立图集 | 彩色 Emoji、OpenType `COLRv1` 矢量彩色字形 | 与单色图集物理隔离，防止 1 个 Emoji 拖垮整屏纯色文本的实例化合批 |
| **GPU 矢量曲线直算** | **无图集**（上传贝塞尔控制点至 GPU SSBO，由 Compute/Fragment 着色器求交计算覆盖率，或大字号 MSDF） | 大字号标题（> 72px）、连续缩放动画、实时可变字体轴动画 | 彻底消除高倍率放大模糊与图集频繁重建抖动（Atlas Thrashing） |

### 懒加载字体回退与可变字体（Variable Fonts）

- **按需懒加载 + 区域优先回退链**：不在冷启动全量解析几十兆 CJK/Emoji 字体表，而是利用码点覆盖位图（Character Map）+ `#Locale` 绑定检索顺序（激活 OpenType `locl` 标签，确保简体中文、繁体中文、日文共用码点时笔画规范正确）。
- **连续设计轴直驱动画**：开放 `wght`（字重）、`wdth`（字宽）与 `opsz`（视觉尺寸自动适配）轴给合成动画与响应式排版（当译文接近容器极限时允许按 `wdth` 微幅收窄，减少生硬的 `...` 省略号截断）。

---

## 3. 光学级渲染质量契约

1. **全链路线性光学空间混合（Linear-Light Blending）**  
   严禁在非线性 sRGB 空间内直接对半透明 UI 与文字抗锯齿边缘做 Alpha 混合（会造成暗边与脏灰过渡带）。所有颜色在进入片元/计算着色器混合前统一解码至线性光学空间（premultiplied linear light），最终输出阶段再按目标曲面格式编码。
2. **感知均匀色彩体系优先（`OKLCH` / `OKLab`）**  
   Silver DSL 与主题系统将 `oklch(l, c, h, a)` 设为默认调色与动画插值空间（编译期可无损转为 `LinearRgba`），确保渐变的中间色与暗色模式自动派生在人眼感知亮度上完全均匀，不会出现传统 HSL/RGB 的亮度塌陷。
3. **逐图层色彩空间与局部 HDR/EDR**  
   `CompositorStyle` 允许逐层附带 `ColorSpace`（`srgb`、`display_p3`、`bt2020_pq`、`extended_srgb`）。Vulkan 主后端对接原生广色域交换链与 DRM 硬件色彩管理直通显示控制器，使 HDR 照片/视频突破 SDR 峰值亮度（EDR）同时保证周围 UI 色彩恒定；WGPU 回退后端必要时在最终合成着色器内执行色调映射（Tone Mapping）。
4. **纹理重采样策略分离**  
   - 静态缩略图/图标：后台线程采用 **Lanczos / Mitchell-Netravali** 带负瓣锐化滤波器高质量降采样，再缓存适配视口的纹理尺寸（防止 4K 原图吃爆显存）。
   - 实时缩放动画图像：GPU 管线自动生成三线性/各向异性 Mipmap 链，杜绝滚动缩放时的摩尔纹闪烁。
5. **时域步调与感知自动化回归**  
   视觉回归 CI 拒绝脆弱的逐像素精确匹配（易受各 GPU 浮点舍入误报），引入基于人类视觉系统（HVS）的感知指标（如 **DSSIM / NVIDIA FLIP**）量化抗锯齿边缘、阴影过渡与线性混合质量。

---

## 4. 零分配国际化（I18n）与 Direct-Passing 效应集成

### 4.1 预编译消息字节码 + 逻辑排版

- **编译期消息表（MessageFormat 2.0）**：翻译资源（含复数形式、性别变格、数字/单位插值）在构建阶段编译为扁平化二进制索引表与驻留字符串切片，运行期查表与格式化零堆分配、微秒级切换。
- **通过 `^attrs.effects.direct_passing` 注入 `#Locale`**：整个组件树无需逐层透传 `locale` 参数；切换语言或局部子树强制 RTL 时，恢复点的 `#Locale` 重绑即可驱动整个子树重新测量与镜像。

```flurry
use silver.i18n.{ Locale, Direction, MessageId }

pub effect enum LocaleCtx {
    ^attrs.effects.direct_passing
    tag() -> str,
    ^attrs.effects.direct_passing
    direction() -> Direction,
    -- format_msg 带 MessageId 实参，属于普通 effect（由安装点外层资源表提供翻译映射）
    format_msg(id: MessageId) -> str,
}

-- 零侵入伪本地化调试处理器（Pseudolocalization）：
-- 无需修改任何业务组件源码，包裹在子树外围即可立即暴露文本截断与硬编码 RTL 缺陷
pub fn with_pseudolocale(child: fn() -> #[LocaleCtx, SceneOps] void) -> #SceneOps void {
    child()# {
        LocaleCtx.tag() => "ar-XA-pseudo",
        LocaleCtx.direction() => Direction.rtl,
        -- 注：format_msg 含参数，不能用 direct_passing，属于普通 effect
        LocaleCtx.format_msg(id) => do {
            let base = Locale.lookup_default(id);
            Locale.pseudo_expand(base, 0.40f32) -- 自动拉长 40% 并添加重音符包裹
        },
    }
}
```

### 4.2 IME 与无障碍协同预留

- **IME 光标锚点同步**：编辑组件将逻辑光标与组合输入区域（Composition Underline）通过 `SceneOps` 写入 `CaretAnchor`；本进程渲染直接定位候选窗，接入 Wayland 扩展后由合成器在同一 VSync 帧内与系统输入法零滞后同步并自动边界翻转。
- **多语言无障碍语义代理**：`Cold` 旁路表保留片段级 `Locale` 标签，使屏幕阅读器在英文主界面朗读到内嵌中文段落时能无缝切换发音引擎。

---

## 5. 待讨论问题（Open Questions）

- **Q6.1（文本塑形底座实现路线）**：在 Flurry 早期生态尚无原生 HarfBuzz/ICU 替代库时，是以最小静态链接/C ABI 包装 `HarfBuzz + SheenBidi / ICU4X` 起步（注意 C 帧仅在界外主栈调用、严禁跨组件挂起边界），还是同步推进纯 Flurry 实现的轻量 UAX 分段/BiDi 子集？
- **Q6.2（`Color` 类型的编译期表示）**：是否在 DSL 中把 `Color` 统一定义为线性空间的 `struct Color { r: f32, g: f32, b: f32, a: f32 }`，而 `oklch(...)` / `srgb(...)` 作为 `pure comptime fn` 构造器在编译期即完成色空间转换（保留运行期动态构造版本），以保证渲染热路径零色空间分支判断？
