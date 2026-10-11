# Google AI Usage Monitor (Antigravity Quota Overlay)

Google Antigravity の内部言語サーバー (`language_server.exe`) と連携し、Google AI Pro (Gemini Models / Claude・GPT) のクレジット残量（Weekly / 5-Hour Burst Limit）を Antigravity ウィンドウ上にスマートに吸着・常駐表示する超軽量・ネイティブな Windows HUD オーバーレイです。

---

## 開発環境のセットアップ (Windows 11)

本プロジェクトの開発環境は、Windows パッケージマネージャー (`winget`) を起点として宣言的に構築できます。

### 1. 基本ツールチェーンの導入

```powershell
# 1. Rust ツールチェーンおよび C リンカー (WinLibs LLVM/GCC) のインストール
winget install --id Rustlang.Rustup -e --accept-source-agreements --accept-package-agreements
winget install --id BrechtSanders.WinLibs.POSIX.UCRT.LLVM -e --accept-source-agreements --accept-package-agreements

# 2. 環境変数の再読み込み
$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path

# 3. Windows GNU ターゲットの導入とデフォルト設定
rustup toolchain install stable-x86_64-pc-windows-gnu
rustup default stable-x86_64-pc-windows-gnu
```

### 2. コード品質維持ツールの導入

コード品質、フォーマット、静的解析、サプライチェーンセキュリティを維持するため、以下の標準ツールを導入します。

```powershell
# 1. rustfmt & clippy (公式ツールチェーンコンポーネント)
rustup component add rustfmt clippy

# 2. 高速バイナリインストーラー (cargo-binstall)
cargo install cargo-binstall --locked

# 3. 品質検査・セキュリティツールの導入
cargo binstall -y cargo-machete cargo-audit cargo-deny
```

| ツール | 用途 |
|---|---|
| **rustfmt** | Rust 公式コードフォーマッター。一貫したコードスタイルの維持 |
| **clippy** | Rust 公式静的解析（Linter）。イディオマティックな書き方やバグ・性能劣化の検出 |
| **cargo-machete** | `Cargo.toml` 内の不要・未使用な依存関係クレートの高速検出 |
| **cargo-audit** | `Cargo.lock` に含まれる依存関係のセキュリティ脆弱性（RustSec）監査 |
| **cargo-deny** | 依存関係のライセンス（SPDX）、アドバイザリ、クレート重複（bans）を一括自動検証 |

---

## 日常の開発ワークフローと品質維持

開発中およびコミット前には、以下のコマンドを実行してコード品質を維持します。

### 個別チェック

```powershell
# コード自動整形
cargo fmt

# フォーマット検証 (CI / コミット前確認)
cargo fmt --check

# 静的解析・リント検証 (警告ゼロを維持)
cargo clippy --all-targets --all-features

# 未使用依存クレートの検査
cargo machete

# 依存クレートのセキュリティ脆弱性監査
cargo audit

# サプライチェーン・ライセンス・重複クレートの包括的検査
cargo deny check
```

### ワンライナーでの一括品質パス

コミット前に以下のコマンドを実行し、すべてのチェックが Green（終了コード 0）であることを確認します。

```powershell
cargo fmt --check; cargo clippy --all-targets --all-features; cargo machete; cargo audit; cargo deny check; cargo test
```

---

## 高速フィードバック・ホットリロード開発 (watch モード)

GUI のデザイン微調整や動作検証時に、コードを変更・保存するたびに自動で再コンパイル・再起動する **ホットリロード開発** が可能です（`cargo-watch` を導入済み）。

別の PowerShell ウィンドウを開き、以下のコマンドを実行したままにしておくことで、コードが修正されるたびにプロセスが自動で再起動し、即座に画面上で結果を確認できます。

```powershell
# コード変更を監視し、コンパイル成功時に即座に再起動 (プロセス停止 -> 再起動)
cargo watch -x run
```

画面収録（キャプチャ）やフィードバックループを回す際にも、このコマンドを常駐させておくことでシームレスに UI の変化を確認できます。

---

## アーキテクチャ

1. **Quota Fetcher (`src/quota.rs`)**:
   - Win32 Toolhelp32Snapshot および PEB コマンドライン直接解析により、実行中の `language_server.exe` から CSRF トークンをセキュアに自動抽出。
   - プロセスのリッスンポート（IPv4 TCP）を特定し、ローカル HTTP Connect-RPC API (`/RetrieveUserQuotaSummary`) から残量パーセンテージを取得。
2. **Window Tracker & Proximity Manager (`src/tracker.rs`, `src/main.rs`)**:
   - `Antigravity.exe` のプロセスツリーを走査してメインウィンドウ（HWND）を特定し、右下に追従。
   - 200ms デバウンスステートマシンにより、ドラッグ操作中の小休止によるフェードインの誤発火やガタつきを完全に防止。
   - **マウス接近自動回避（Mouse Proximity Fade-out）**: HUDの領域またはその近傍（外側 +20px）にマウスカーソルが入ると、背後のUI操作を妨げないよう即座にスッと自動フェードアウトし、マウスが離れると自然に復帰。
3. **High-DPI Vector HUD Overlay (`src/overlay.rs`)**:
   - `tiny-skia`（2D ソフトウェアベクターラスタライザー）と `cosmic-text`（高品位フォントレンダリング）による滑らかなタコメーター風デザイン。
   - `UpdateLayeredWindow` + 32-bit DIBSection キャッシュを採用し、角丸外側まで 100% 完全透過（Per-pixel Alpha Blending）。
   - **完全クリックスルー (`WS_EX_TRANSPARENT`)**: HUDが表示されている最中でも、背後にあるエディタやターミナル、ボタンへのマウスクリックやドラッグを一切阻害せず直下に透過。
   - フェード中はベクター再描画を行わず、Windows DWM (Desktop Window Manager) の GPU 合成にアルファ値のみを渡すため、フェード中の CPU 使用率は実質 0%。

---

## ビルドと実行

```powershell
# デバッグ実行 (独立した外部 PowerShell から実行)
cargo run

# リリースビルド (省メモリ・最適化された単一バイナリ)
cargo build --release
```
