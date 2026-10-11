# Google AI Usage Monitor (Antigravity Quota Overlay)

Google Antigravity の内部言語サーバー (`language_server.exe`) と連携し、Google AI Pro (Gemini Models / Claude・GPT) のクレジット残量（Weekly / 5-Hour Limit）を Antigravity ウィンドウ上にスマートにオーバーレイ表示する軽量・ネイティブな常駐ツールです。

## 開発環境のセットアップ (Windows 11)

本プロジェクトの開発環境は、Windows パッケージマネージャー (`winget`) を起点として宣言的に構築できます。

```powershell
# 1. Rust ツールチェーンのインストール
winget install --id Rustlang.Rustup -e --accept-source-agreements --accept-package-agreements

# 2. 環境変数の再読み込み (または新しいターミナルを開く)
$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path

# 3. 動作確認
rustc --version
cargo --version
```

## アーキテクチャ

1. **Quota Fetcher (`src/quota.rs`)**:
   - `sysinfo` および Win32 API を用いて実行中の `language_server.exe` から CSRF トークンを自動抽出。
   - プロセスのリッスンポートをスキャンし、ローカル HTTP Connect-RPC API (`RetrieveUserQuotaSummary`) から残量・リセット日時を取得。
2. **Window Tracker (`src/tracker.rs`)**:
   - Win32 API (`SetWinEventHook`) による完全イベント駆動型のウィンドウ位置追従。
   - Antigravity ウィンドウの移動・リサイズ・最小化・可視状態に同期。
3. **UI Overlay (`src/main.rs`)**:
   - タコメーター風のゲージデザインによる視覚的モニタリング。
   - 移動・リサイズ時のフェードアウト/フェードインによるスマートなUX。

## ビルドと実行

```powershell
# デバッグ実行
cargo run

# リリースビルド (最適化された単一バイナリ)
cargo build --release
```
