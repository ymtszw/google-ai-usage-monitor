---
name: gui-dimensionless-design
description: >-
  Apply dimensionless scale design principles for GUI spacing, margins, padding, and typography.
  Use whenever designing, refining, styling, or reviewing user interfaces across any platform
  (Rust/Win32/egui/Slint, Web/HTML/CSS, Flutter, React/React Native, Tauri, etc.) to calculate
  responsive spacing scales from Base_Space, typography scales from Base_Text, line heights,
  and element relationships without hardcoding absolute pixels.
---

# GUI Design: Dimensionless Token System (Spacing & Typography)

This skill provides a platform-agnostic, dimensionless design system for user interfaces.
All sizes, margins, paddings, and typography must derive from two dimensionless base tokens (`Base_Space` and `Base_Text`) rather than hardcoded absolute units (px, dp, pt).

---

## 1. Base Tokens Definition

All dimensions and typography are calculated as multipliers relative to two base tokens (scale = 1.0):

* **`Base_Space` (Spacing Base)**:
  * Default layout interval.
  * Adjusts according to UI density settings:
    * *Compact*: e.g., 6px ~ 8px (dashboards, dense data tools, overlays)
    * *Comfortable*: e.g., 10px ~ 12px (content applications, broad forms)
* **`Base_Text` (Typography Base)**:
  * Default body text size.
  * Adjusts according to system font preferences, DPI scale, and accessibility zoom (e.g., 12px ~ 14px).

---

## 2. Spacing Scale (Relative to `Base_Space`)

All layout margins, paddings, and gaps must strictly follow the multiplier scale:

| Multiplier | Scale Name | Usage / Intent | Typical Examples |
|---|---|---|---|
| **`0.25x`** (`Base_Space * 0.25`) | 極小 (X-Small) | 微細な間隔 | アイコンとテキストの間隔、バッジ・タグの内部余白 |
| **`0.5x`** (`Base_Space * 0.5`) | 小 (Small) | 構造的セット内 | タイトルとサブタイトル、同一カード内の密接な要素間 |
| **`1.0x`** (`Base_Space * 1.0`) | 中 (Medium) | 標準間隔 | コントロール内パディング、リストアイテムの間隔、段落間 |
| **`1.5x`** (`Base_Space * 1.5`) | 大 (Large) | コンポーネント区切り | パネル・カードの内部パディング、独立したグループ間の余白 |
| **`2.0x`** (`Base_Space * 2.0`) | 特大 (X-Large) | セクション境界 | 異なるコンテンツエリア・セクション間のマージン |
| **`3.0x` / `4.0x`** | 巨大 (XX-Large) | 大区切り | 初期起動画面（Welcome画面）、ビュー間の大余白 |

---

## 3. Typography Scale (Relative to `Base_Text`)

### ① Font Size Factors

Font sizes are calculated strictly relative to `Base_Text`:

| Level | Size Factor | Weight | Role & Usage |
|---|---|---|---|
| **H1** (大見出し / View Title) | `1.5x` 〜 `2.0x` | Bold / Heavy | 画面・ビューの最上位タイトル |
| **H2** (中見出し / Section Title) | `1.25x` | SemiBold / Bold | パネル、カードグループのセクションヘッダー |
| **Body** (本文 / Standard Text) | `1.0x` | Regular / Medium | 標準本文、コントロール内ラベル、数値表示 |
| **Caption** (注記 / Property Label) | `0.75x` 〜 `0.85x` | Regular / Medium | メーターラベル、補足情報、タイムスタンプ、ヘルプ |

### ② Line Height Factor (Relative to Own Font Size)

Line height is a dimensionless multiplier relative to each text element's *own font size*:

* **Body (`1.5` 〜 `1.7`)**: 文章・複数行コンテンツの可読性を確保するため、ゆとりを持たせる。
* **Heading (`1.2` 〜 `1.3`)**: 見出しは文字が大きく、複数行時に間伸びして見えるのを防ぐため引き締める。

### ③ Line Length Constraint (Reading Width)

* 複数行の文章・テキストコンテナは、視線移動の負荷を抑えるため、**文字サイズ（自己サイズ）の 35倍 〜 40倍（35ch 〜 40ch）** を最大幅（`max-width`）とする。

---

## 4. Spacing × Typography Relationship Rules

視覚的なグルーピング（近接の原則 / ゲシュタルト心理学）を維持するための関係性ルール：

```text
[ 前の領域 / コンテンツ ]
       │
       ▼ Margin: 2.0x (Base_Space * 2.0)  <-- 「話の文脈が変わる」明確な境界
[ 次の見出し (Heading) ]
       │
       ▼ Margin: 0.5x (Base_Space * 0.5)  <-- 見出しと本文が「1つの構造セット」に見える密着度
[ 本文 / コンテンツ (Body) ]
       │
       ▼ Margin: 1.0x (Base_Space * 1.0)  <-- 段落同士の独立性を保つ標準余白
[ 次の段落 (Body) ]
```

| 要素の組み合わせ | 推奨される間隔 (Margin / Gap) | 設計の意図 |
|---|---|---|
| **見出し** と その直後の要素 | `0.5x` (`Base_Space * 0.5`) | 2つで「1つの構造的セット」に見えるよう密着させる |
| **段落** と 次の段落 (本文同士) | `1.0x` (`Base_Space * 1.0`) | 行高（内部余白）より広い外余白を確保し、独立性を出す |
| **前の領域** と 次の見出し | `2.0x` (`Base_Space * 2.0`) | 話のテーマ・コンテキストが変わることを示す明確な境界 |

---

## 5. Agent Constraints & Implementation Patterns

### 🚨 Mandatory Agent Rules

1. **絶対値コード出力の禁止**:
   プラットフォーム固有のコードを出力する際、数値をハードコード（例: `padding: 16px`, `margin: 24`）してはならない。必ず定義したデザイントークン定数・構造体・関数・CSS変数等を経由してレイアウトを導出すること。
2. **連動スケールの維持**:
   ウィンドウサイズ変更、高DPI対応、UI密度モード（Compact / Comfortable）切り替え時には、`Base_Space` と `Base_Text` の基本値のみを切り替え、各乗数（0.25x, 0.5x, 1.0x, 1.25x 等）の比率は頑なに維持すること。
3. **階層の逆転禁止**:
   外部マージンが内部パディングや行間を下回るような、近接の原則（ゲシュタルト崩壊）を起こす設定を絶対に出力してはならない（例: セクション間余白が段落間余白より小さくなる等）。

---

### Implementation Examples by Platform

#### Rust (Native UI, Win32, tiny-skia, egui)

```rust
pub struct DesignTokens {
    pub base_space: f32,
    pub base_text: f32,
}

impl DesignTokens {
    pub const COMPACT: Self = Self { base_space: 8.0, base_text: 12.0 };
    pub const COMFORTABLE: Self = Self { base_space: 12.0, base_text: 14.0 };

    // Spacing scale
    #[inline] pub fn space_xs(&self) -> f32 { self.base_space * 0.25 }
    #[inline] pub fn space_sm(&self) -> f32 { self.base_space * 0.5 }
    #[inline] pub fn space_md(&self) -> f32 { self.base_space * 1.0 }
    #[inline] pub fn space_lg(&self) -> f32 { self.base_space * 1.5 }
    #[inline] pub fn space_xl(&self) -> f32 { self.base_space * 2.0 }

    // Typography scale
    #[inline] pub fn text_h1(&self) -> f32 { self.base_text * 1.75 }
    #[inline] pub fn text_h2(&self) -> f32 { self.base_text * 1.25 }
    #[inline] pub fn text_body(&self) -> f32 { self.base_text * 1.0 }
    #[inline] pub fn text_caption(&self) -> f32 { self.base_text * 0.8 }

    // Line heights
    #[inline] pub fn line_height_body(&self) -> f32 { self.text_body() * 1.6 }
    #[inline] pub fn line_height_heading(&self) -> f32 { self.text_h2() * 1.25 }
}
```

#### CSS / Web / Tauri

```css
:root {
  --base-space: 8px; /* Compact: 8px, Comfortable: 12px */
  --base-text: 13px;

  /* Spacing Scale */
  --space-xs: calc(var(--base-space) * 0.25);
  --space-sm: calc(var(--base-space) * 0.5);
  --space-md: calc(var(--base-space) * 1.0);
  --space-lg: calc(var(--base-space) * 1.5);
  --space-xl: calc(var(--base-space) * 2.0);

  /* Typography Scale */
  --text-h1: calc(var(--base-text) * 1.75);
  --text-h2: calc(var(--base-text) * 1.25);
  --text-body: calc(var(--base-text) * 1.0);
  --text-caption: calc(var(--base-text) * 0.8);

  /* Line Heights */
  --lh-body: 1.6;
  --lh-heading: 1.25;

  /* Line Length */
  --max-line-width: 40ch;
}
```
