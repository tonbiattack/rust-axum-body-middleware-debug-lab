# デバッグ記録: 監査ミドルウェアがJSON APIのボディを消費する

## 目的

Rust 1.75.0とAxum 0.7で、JSONリクエストを監査するミドルウェアがボディを消費した後に空のボディを渡すことで、ハンドラーが有効なJSONを受け取れなくなる理由を、HTTP境界の最小例で確認します。

> 契約: `POST /tasks`へ有効なJSONを送った場合、`201 Created`と作成済み件数1を得る。バグ状態では`400 Bad Request`と作成済み件数0になる。

## 実行環境と再現境界

| 項目 | 内容 |
| --- | --- |
| 言語処理系 | Rust 1.75.0 |
| 難易度プロファイル | 実践・上級。HTTP応答、ログ、ハンドラー実行有無、状態更新を分けて観測するため |
| ビルド・テスト方法 | `cargo test -- --nocapture` |
| 使用する依存関係 | Axum 0.7、Tokio、Towerの`oneshot` |
| 使用しないもの | 実ネットワーク、DB、外部API、時刻や待機に依存する同期 |
| 公開境界 | Axum `Router`に`POST /tasks`を送るルーター境界 |
| 最終観測 | HTTPステータスと`AtomicUsize`の作成済み件数 |
| 決定性の確保 | 固定されたJSON・ヘッダー・インメモリ状態だけを用いる |

この境界を選んだ理由は、ブラウザやWebサーバーを起動せず、ミドルウェアから`Json`抽出器までの実際のHTTP処理順を直接観測できるためです。

## 最初に観測した事実

| 観測順 | 事実 | 得られた証拠 |
| --- | --- | --- |
| 1 | 有効なJSONと`Content-Type: application/json`を送った。 | テストで`{"title":"請求書を確認する"}`とヘッダーを固定 |
| 2 | 監査ミドルウェアは36バイトのボディを読んだ。 | `[audit] path=/tasks body_bytes=36 を監査しました`というログ |
| 3 | 応答は`400 Bad Request`だった。 | ルーター境界で取得した`response.status()` |
| 4 | ハンドラーの作成ログは出ず、作成済み件数は0だった。 | `created_task_count()`の最終観測 |

バグ状態のコミットは`18a8b67`です。次のコマンドを実行すると、設定やコンパイルではなく、意図したHTTP応答と最終状態の差分で失敗します。

```bash
cargo test json_post_must_reach_handler_after_audit_middleware -- --nocapture
```

実際の失敗は次のとおりでした。

```text
[audit] path=/tasks body_bytes=36 を監査しました
監査済みのJSON POSTは201と作成済み件数1を返す必要があります: status=400 Bad Request, created_task_count=0
```

GDBでバグ状態の`src/lib.rs:65`にブレークポイントを置くと、`Request::from_parts(parts, Body::empty())`を実行して次の処理へ渡す直前に停止しました。スタック先頭は`rust_axum_body_middleware_debug_lab::audit_json_body::{async_fn#0}`でした。監査ログが36バイトを示した後にこの行へ到達しているため、JSONは途中で消えたのではなく、ミドルウェアが空のボディを明示的に再構築していることを確認しました。

```bash
test_bin=$(find target/debug/deps -maxdepth 1 -type f -executable -name 'rust_axum_body_middleware_debug_lab-*' | head -n 1)
gdb --batch \
  -ex 'break src/lib.rs:65' \
  -ex 'run --exact tests::json_post_must_reach_handler_after_audit_middleware --nocapture' \
  -ex 'bt 6' \
  -ex 'info locals' \
  --args "$test_bin"
```

## 競合仮説と検証

| 仮説 | 予測 | 検証 | 結果 |
| --- | --- | --- | --- |
| 監査ミドルウェアがボディを消費している | 監査ログは出るが、ハンドラーはJSONを読めず400になる | ボディを読むコード、GDBの停止位置、HTTP応答を確認する | 支持 |
| JSONまたはContent-Typeが不正である | ミドルウェアの有無にかかわらず400になる | 有効なJSONと`application/json`を固定し、修正後に201になることを確認する | 除外 |
| ハンドラー内の状態更新が失敗している | ハンドラーの作成ログが出るが、件数だけが0になる | ハンドラーログと作成済み件数を確認する | 除外 |

## 確定した原因

Axumのリクエストボディは非同期ストリームで一度しか消費できず、`Json`抽出器もボディを消費してデシリアライズします。[1] [2] バグ実装は`to_bytes(body, ...)`で元の`Body`を`Bytes`へ消費した後、`Body::empty()`を使ってリクエストを再構築していました。

そのため、監査自体は元のJSONを読めますが、後段の`Json<CreateTask>`には空のボディだけが渡ります。`Json`抽出器は構文として有効なJSONがない場合にリクエストを拒否するため、ハンドラーは呼ばれず`400 Bad Request`になります。[2]

この結論は、ラボで観測した監査ログ・GDB停止位置・HTTP応答・状態と、Axum公式ドキュメントおよび公式サンプルの再構築手順の両方で裏づけています。[1] [2] [3]

## 最小修正

`to_bytes`で得た`bytes`を監査した後、`Body::from(bytes)`で新しいボディを作り、元のリクエストパーツと組み合わせて次へ渡します。

```rust
next.run(Request::from_parts(parts, Body::from(bytes))).await
```

この修正は、ボディを消費して空のボディを渡す直接原因だけを変更します。監査内容、ルーティング、APIレスポンス形式、依存関係、状態管理方式は変えていません。修正コミットは`1777227`です。

## 回帰保証

| 守ること | テストまたは診断 | 修正後の結果 |
| --- | --- | --- |
| 有効なJSON POSTがハンドラーへ届く | `json_post_must_reach_handler_after_audit_middleware` | `201 Created`と作成済み件数1を確認して成功 |
| 不正JSONはハンドラーを呼ばない | `syntactically_invalid_json_is_rejected_without_state_change` | `400 Bad Request`と作成済み件数0を確認して成功 |
| コード形式が標準に従う | `cargo fmt --check` | 成功 |

固定済みの状態で`cargo test -- --nocapture`を実行し、ユニットテスト2件とドキュメントテストがすべて成功することを確認しました。

## 再現手順

```bash
# 修正済み状態を検証する
cargo fmt --check
cargo test -- --nocapture

# バグ状態を確認する。作業中の変更は先に退避する
git switch --detach 18a8b67
cargo test json_post_must_reach_handler_after_audit_middleware -- --nocapture

# 修正済み状態へ戻る
git switch main
```

## スコープと注意点

このラボは、最大64KiBのJSONボディをメモリへバッファリングして監査する条件だけを扱います。長時間続くストリームや大容量アップロードを同じ方法でバッファリングしてはいけません。公式サンプルも、ボディが長時間継続するストリームではこの方法を使えないと明記しています。[3]

また、実運用の監査ログでは、認証情報や個人情報をそのまま出力しないように、フィールド単位のマスキング、出力制限、アクセス制御を別途設計する必要があります。

## References

[1] [Axum extractors documentation](https://docs.rs/axum/latest/axum/extract/index.html)

[2] [Axum `Json` documentation](https://docs.rs/axum/latest/axum/struct.Json.html)

[3] [Axum公式のボディ消費ミドルウェア例](https://github.com/tokio-rs/axum/blob/main/examples/consume-body-in-extractor-or-middleware/src/main.rs)
