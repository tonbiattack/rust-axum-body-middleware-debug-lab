# Axumの監査ミドルウェアがJSON APIのボディを消費する

RustとAxumで、JSONボディを監査するミドルウェアがハンドラーへ空のボディを渡し、正しいJSONリクエストを`400 Bad Request`にしてしまう不具合を再現します。失敗するAPI契約テスト、実行ログ、GDB、コードリーディング、最小修正、回帰テストを順に確認するデバッグラボです。

## この題材で守る契約

> `POST /tasks`へ有効な`application/json`を送った場合、監査ミドルウェアを通過しても`201 Created`を返し、タスク作成の状態を1件増やします。

バグ状態では、監査ミドルウェアがリクエストボディを読み取った後に空の`Body`を渡すため、`Json`抽出器が`400 Bad Request`を返し、ハンドラーは状態を更新しません。Axumのリクエストボディは非同期ストリームで一度しか消費できず、`Json`抽出器もボディを消費します。[1] [2]

## 最短の開始手順

修正済みの既定ブランチで、次を実行します。

```bash
cargo fmt --check
cargo test -- --nocapture
```

有効なJSON POSTが`201 Created`と状態更新を返すテスト、不正JSONが`400 Bad Request`で状態を変えないテストが成功します。

## バグを再現する

バグ状態はコミット`18a8b67`に保存しています。作業中の変更を退避してから、次を実行します。

```bash
git switch --detach 18a8b67
cargo test json_post_must_reach_handler_after_audit_middleware -- --nocapture
```

テストは、監査ログがJSONを読んだ後に`400 Bad Request`を返し、作成済み件数が0のままであることを示して失敗します。確認後は修正済みブランチへ戻ります。

```bash
git switch main
```

## 観測の要約

| 観測点 | バグ状態 | 修正後 |
| --- | --- | --- |
| 監査ログ | JSONのバイト数を出力する | JSONのバイト数を出力する |
| HTTP応答 | `400 Bad Request` | `201 Created` |
| ハンドラーのログ | 出力されない | タスク作成ログが出力される |
| 作成済み件数 | 0 | 1 |

詳細な仮説、証拠、原因、修正、回帰保証は[`docs/debugging-record.md`](docs/debugging-record.md)に記録しています。題材と再現設計は[`docs/topic-brief.md`](docs/topic-brief.md)を参照してください。

## 構成

```text
src/lib.rs                  Axum API、監査ミドルウェア、契約テスト
README.md                   開始手順
docs/topic-brief.md         題材と再現設計
docs/debugging-record.md    調査記録
```

## 前提条件

| 項目 | バージョンまたは条件 |
| --- | --- |
| Rust | 1.75以上 |
| Cargo | Rust同梱の標準ビルド・テストツール |
| Axum | `Cargo.toml`で指定された0.7系 |
| 外部サービス | 不要 |

## スコープ

このラボは、リクエストボディをいったんバッファリングするAxumミドルウェアと、後段の`Json`抽出器の組み合わせだけを扱います。大きなストリーミングボディ、監査ログへの機密情報の出力、Webサーバーのネットワーク設定、実永続化層のトランザクションは扱いません。

## References

[1] [Axum extractors documentation](https://docs.rs/axum/latest/axum/extract/index.html)

[2] [Axum `Json` documentation](https://docs.rs/axum/latest/axum/struct.Json.html)

[3] [Axum公式のボディ消費ミドルウェア例](https://github.com/tokio-rs/axum/blob/main/examples/consume-body-in-extractor-or-middleware/src/main.rs)
