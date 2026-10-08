/**
 * One long reply in several scripts, with every block a reply can hold, so a
 * reader sees the rhythm of real reading text: Latin with and without
 * diacritics, Cyrillic, long German compounds, and Chinese, Japanese and
 * Korean set beside Latin words, numbers and code.
 */
export const LONG_TEXT_LANGUAGES = ['en', 'zh', 'ja', 'ko', 'ru', 'de', 'vi'] as const
export type LongTextLanguage = (typeof LONG_TEXT_LANGUAGES)[number]

export const LONG_TEXT_NAMES: Record<LongTextLanguage, string> = {
  en: 'English',
  zh: '简体中文',
  ja: '日本語',
  ko: '한국어',
  ru: 'Русский',
  de: 'Deutsch',
  vi: 'Tiếng Việt',
}

const code = `\`\`\`ts
export async function retry<T>(run: () => Promise<T>, attempts = 3): Promise<T> {
  for (let attempt = 1; ; attempt++) {
    try {
      return await run()
    } catch (error) {
      if (attempt === attempts)
        throw error
      await new Promise((resolve) => setTimeout(resolve, 250 * 2 ** attempt))
    }
  }
}
\`\`\`

\`\`\`
[sync] heartbeat late by 5.2 s, closing socket 7f3a
[sync] reconnecting (attempt 1, waiting 500 ms)
[sync] connected; replaying from revision 18342
\`\`\``

// A table far wider than the column, which scrolls on its own.
const wide = `| Region | Endpoint | p50 | p95 | p99 | Reconnects / h | Late pongs / h | Last incident | Notes |
| --- | --- | --: | --: | --: | --: | --: | --- | --- |
| eu-central-1 | \`wss://eu1.sync.example.com/v2/socket\` | 38 ms | 112 ms | 340 ms | 1.2 | 0.4 | 2026-09-14 03:12 UTC | Load balancer drains idle sockets after 350 s |
| us-east-1 | \`wss://us1.sync.example.com/v2/socket\` | 41 ms | 128 ms | 410 ms | 2.8 | 1.1 | 2026-09-30 17:45 UTC | Corporate proxies cut WebSockets after 60 s of silence |
| ap-southeast-1 | \`wss://ap1.sync.example.com/v2/socket\` | 66 ms | 240 ms | 910 ms | 5.3 | 3.7 | 2026-10-02 08:20 UTC | Mobile networks switch towers often |`

export const LONG_TEXT: Record<LongTextLanguage, string> = {
  en: `# Why the sync stalls after a laptop wakes up

I traced the stall to the reconnect path in \`sync/socket.ts\`. When the laptop sleeps, the operating system keeps the TCP connection open on paper, so the client never sees a close event; when it wakes, the first write sits in the kernel's buffer for up to **two minutes** before the connection is declared dead. During that time the page shows the last state it received and looks frozen, even though nothing on the server is wrong. The fix is to treat a missed heartbeat as a lost connection instead of waiting for the operating system to tell us.

## What changes

The client already sends a ping every 15 seconds, but it never checked whether the matching pong arrived. Now it does, and it acts on the answer:

- A pong that is more than **5 seconds** late closes the socket and starts the normal reconnect, which replays everything after the last revision the page saw.
- A page that becomes visible again (\`visibilitychange\`) sends a ping at once rather than waiting for the next tick, so a laptop that just woke up finds out within a second whether its connection survived the night.
- Reconnect attempts back off exponentially, starting at 250 ms and capped at 30 seconds, so a phone in a tunnel doesn't drain its battery.
  - The backoff resets after a connection has stayed healthy for a full minute.
  - A manual **Reload** skips the backoff entirely.

## How to verify it

1. Open a conversation and start a long-running command, for example \`sleep 600 && echo done\`.
2. Put the machine to sleep for at least five minutes, then wake it.
3. Watch the status line: it should read *Reconnecting…* for under a second and then show the command's new output without a reload.

> A heartbeat only proves the path was alive a moment ago. It cannot prove the next message will arrive, which is why every message still carries its revision and the server still fills any gap.

### The retry helper

The reconnect loop uses a small helper that the upload code can share:

${code}

| Case | Before | After |
| --- | --- | --- |
| Laptop wakes from sleep | Frozen for up to 2 min | Recovers in about 1 s |
| Wi-Fi switches networks | Frozen until the next write fails | Recovers on the next missed pong |
| Server restarts | Recovers on close | Unchanged |

${wide}

- [x] Detect a late pong and reconnect
- [x] Ping when the page becomes visible
- [ ] Show how long the page has been offline in the status line

---

None of this changes the protocol, so an older server keeps working with the new client. If you see the page freeze again after this lands, the log line \`heartbeat late by …\` will tell us whether the pong was late or the reconnect itself was slow, and that is the first thing I would look at.`,

  zh: `# 笔记本唤醒后同步卡住的原因

我把问题定位到了 \`sync/socket.ts\` 里的重连逻辑。笔记本休眠时，操作系统名义上仍然保持着 TCP 连接，所以客户端收不到任何关闭事件；唤醒之后，第一次写入会在内核缓冲区里停留**长达两分钟**，系统才判定连接已断开。这段时间里页面一直显示收到的最后一个状态，看起来像是卡死了，而服务器那边其实一切正常。修复思路是：错过一次心跳就当作连接已经丢失，而不是等操作系统来告诉我们。

## 改了什么

客户端本来每 15 秒发一次 ping，但从来没有检查对应的 pong 有没有回来。现在会检查，并且根据结果行动：

- pong 迟到超过 **5 秒**，就关闭 socket 并走正常的重连流程，重放页面看到的最后一个版本之后的全部内容。
- 页面重新可见时（\`visibilitychange\`）立刻发一次 ping，不再等下一个周期。刚唤醒的笔记本一秒之内就能知道连接是否还活着。
- 重连按指数退避，从 250 ms 开始，最长 30 秒，这样在隧道里的手机不会把电耗光。
  - 连接稳定保持一整分钟后，退避时间重新计算。
  - 手动点击 **Reload** 会直接跳过退避。

## 怎么验证

1. 打开一个对话，运行一条耗时很长的命令，比如 \`sleep 600 && echo done\`。
2. 让电脑休眠至少五分钟，然后唤醒。
3. 观察状态栏：它应该显示「正在重新连接…」不到一秒，然后不用刷新就能看到命令的新输出。

> 心跳只能证明这条路径刚才还是通的，不能保证下一条消息一定送达。所以每条消息仍然带着它的版本号，服务器也仍然会补齐中间缺失的部分。

### 重试辅助函数

重连循环用了一个小小的辅助函数，上传代码也可以复用：

${code}

| 场景 | 修改前 | 修改后 |
| --- | --- | --- |
| 笔记本从休眠中唤醒 | 最长卡住 2 分钟 | 大约 1 秒恢复 |
| Wi-Fi 切换网络 | 卡到下一次写入失败为止 | 下一次 pong 迟到时恢复 |
| 服务器重启 | 收到关闭事件后恢复 | 不变 |

${wide}

- [x] 检测迟到的 pong 并重连
- [x] 页面可见时发送 ping
- [ ] 在状态栏显示页面已离线多久

---

这些改动都没有修改协议，所以旧版服务器配合新客户端照样能用。如果合并之后页面又卡住了，日志里的 \`heartbeat late by …\` 会告诉我们究竟是 pong 迟到了，还是重连本身太慢——这是我会首先查看的地方。`,

  ja: `# ノート PC のスリープ復帰後に同期が止まる理由

原因は \`sync/socket.ts\` の再接続処理にありました。ノート PC がスリープしている間、OS は形式上 TCP 接続を開いたままにするため、クライアントには切断イベントが一切届きません。復帰後の最初の書き込みはカーネルのバッファに**最大 2 分間**とどまり、そのあとでようやく接続が切れたと判定されます。その間ページは最後に受け取った状態を表示し続けるので、サーバー側には何の問題もないのに固まったように見えます。修正方針は、OS からの通知を待たずに、ハートビートを 1 回取りこぼした時点で接続が失われたとみなすことです。

## 変更点

クライアントはもともと 15 秒ごとに ping を送っていましたが、対応する pong が届いたかどうかは確認していませんでした。今回からは確認し、その結果に応じて動きます。

- pong が **5 秒**以上遅れたらソケットを閉じ、通常の再接続を始めます。ページが最後に見たリビジョン以降の内容はすべて再送されます。
- ページが再び表示されたとき（\`visibilitychange\`）は次の周期を待たずにすぐ ping を送るので、復帰直後のノート PC でも 1 秒以内に接続の生死がわかります。
- 再接続は 250 ms から始まり最大 30 秒まで指数的に間隔を広げるので、トンネルの中のスマートフォンが電池を使い果たすことはありません。
  - 接続が 1 分間安定すると、待ち時間はリセットされます。
  - 手動の **Reload** は待ち時間を飛ばします。

## 確認手順

1. 会話を開き、\`sleep 600 && echo done\` のような長時間かかるコマンドを実行します。
2. マシンを 5 分以上スリープさせてから復帰させます。
3. ステータス行を確認します。「再接続中…」の表示が 1 秒未満で消え、再読み込みなしでコマンドの新しい出力が表示されるはずです。

> ハートビートが証明できるのは、経路がついさっきまで生きていたことだけです。次のメッセージが届くことまでは保証できないので、すべてのメッセージには引き続きリビジョンが付き、抜けた部分はサーバーが埋めます。

### リトライ用ヘルパー

再接続ループでは、アップロード処理とも共有できる小さなヘルパーを使っています。

${code}

| ケース | 変更前 | 変更後 |
| --- | --- | --- |
| スリープからの復帰 | 最大 2 分間固まる | 約 1 秒で回復 |
| Wi-Fi のネットワーク切り替え | 次の書き込みが失敗するまで固まる | 次の pong の遅れで回復 |
| サーバーの再起動 | 切断イベントで回復 | 変わらず |

${wide}

- [x] 遅れた pong を検出して再接続する
- [x] ページ表示時に ping を送る
- [ ] オフラインだった時間をステータス行に表示する

---

プロトコルは一切変えていないので、古いサーバーでも新しいクライアントはそのまま動きます。もしこの変更のあとでまたページが固まったら、ログの \`heartbeat late by …\` を見れば、pong が遅れたのか再接続そのものが遅かったのかがわかります。まずはそこを確認するつもりです。`,

  ko: `# 노트북이 절전에서 깨어난 뒤 동기화가 멈추는 이유

원인은 \`sync/socket.ts\`의 재연결 경로에 있었습니다. 노트북이 절전 상태일 때 운영체제는 형식상 TCP 연결을 열어 둔 채로 두기 때문에 클라이언트는 연결 종료 이벤트를 전혀 받지 못합니다. 깨어난 뒤 첫 번째 쓰기는 커널 버퍼에 **최대 2분** 동안 머문 다음에야 연결이 끊어진 것으로 판정됩니다. 그동안 페이지는 마지막으로 받은 상태를 계속 보여 주므로, 서버에는 아무 문제가 없는데도 멈춘 것처럼 보입니다. 해결 방법은 운영체제가 알려 주기를 기다리지 않고, 하트비트를 한 번 놓치면 연결이 끊긴 것으로 보는 것입니다.

## 바뀐 점

클라이언트는 원래 15초마다 ping을 보냈지만, 그에 맞는 pong이 왔는지는 확인하지 않았습니다. 이제는 확인하고 그 결과에 따라 움직입니다.

- pong이 **5초** 넘게 늦으면 소켓을 닫고 일반 재연결을 시작합니다. 페이지가 마지막으로 본 리비전 이후의 내용은 모두 다시 전송됩니다.
- 페이지가 다시 보이게 되면(\`visibilitychange\`) 다음 주기를 기다리지 않고 바로 ping을 보내므로, 방금 깨어난 노트북도 1초 안에 연결이 살아 있는지 알 수 있습니다.
- 재연결 간격은 250 ms에서 시작해 최대 30초까지 지수적으로 늘어나므로, 터널 안의 휴대폰이 배터리를 다 쓰지 않습니다.
  - 연결이 1분 동안 안정적으로 유지되면 간격이 초기화됩니다.
  - 수동 **Reload**는 대기 시간을 건너뜁니다.

## 확인 방법

1. 대화를 열고 \`sleep 600 && echo done\`처럼 오래 걸리는 명령을 실행합니다.
2. 컴퓨터를 5분 이상 절전 상태로 두었다가 깨웁니다.
3. 상태 표시줄을 봅니다. 「다시 연결하는 중…」이 1초도 안 되어 사라지고, 새로고침 없이 명령의 새 출력이 나타나야 합니다.

> 하트비트가 증명하는 것은 경로가 방금 전까지 살아 있었다는 사실뿐입니다. 다음 메시지가 도착한다는 보장은 없기 때문에, 모든 메시지는 여전히 리비전을 가지고 있고 빠진 부분은 서버가 채웁니다.

### 재시도 도우미

재연결 루프는 업로드 코드와도 함께 쓸 수 있는 작은 도우미 함수를 사용합니다.

${code}

| 상황 | 변경 전 | 변경 후 |
| --- | --- | --- |
| 노트북이 절전에서 깨어남 | 최대 2분 동안 멈춤 | 약 1초 만에 복구 |
| Wi-Fi 네트워크 전환 | 다음 쓰기가 실패할 때까지 멈춤 | 다음 pong 지연 시 복구 |
| 서버 재시작 | 종료 이벤트로 복구 | 변화 없음 |

${wide}

- [x] 늦은 pong을 감지하고 다시 연결하기
- [x] 페이지가 보일 때 ping 보내기
- [ ] 오프라인이었던 시간을 상태 표시줄에 보여 주기

---

프로토콜은 전혀 바꾸지 않았으므로 이전 서버에서도 새 클라이언트가 그대로 동작합니다. 이 변경 이후에도 페이지가 다시 멈춘다면, 로그의 \`heartbeat late by …\` 줄을 보면 pong이 늦었는지 재연결 자체가 느렸는지 알 수 있습니다. 제가 가장 먼저 확인할 곳도 그 부분입니다.`,

  ru: `# Почему синхронизация замирает после пробуждения ноутбука

Я нашёл причину в логике переподключения в \`sync/socket.ts\`. Пока ноутбук спит, операционная система формально держит TCP-соединение открытым, поэтому клиент не получает события закрытия; после пробуждения первая запись может пролежать в буфере ядра **до двух минут**, прежде чем соединение будет признано мёртвым. Всё это время страница показывает последнее полученное состояние и выглядит зависшей, хотя на сервере всё в порядке. Исправление — считать пропущенный heartbeat потерей соединения, а не ждать, пока об этом сообщит операционная система.

## Что меняется

Клиент и раньше отправлял ping каждые 15 секунд, но никогда не проверял, пришёл ли ответный pong. Теперь проверяет и действует по результату:

- Если pong опаздывает больше чем на **5 секунд**, клиент закрывает сокет и запускает обычное переподключение, которое повторно отправляет всё, что пришло после последней ревизии, увиденной страницей.
- Когда страница снова становится видимой (\`visibilitychange\`), клиент сразу отправляет ping, не дожидаясь следующего такта, так что только что проснувшийся ноутбук меньше чем за секунду узнаёт, пережило ли соединение ночь.
- Интервал между попытками растёт экспоненциально — от 250 мс до максимума в 30 секунд, — поэтому телефон в тоннеле не разряжает батарею.
  - Интервал сбрасывается, когда соединение остаётся стабильным целую минуту.
  - Ручной **Reload** пропускает ожидание.

## Как проверить

1. Откройте разговор и запустите долгую команду, например \`sleep 600 && echo done\`.
2. Переведите компьютер в сон хотя бы на пять минут, затем разбудите его.
3. Следите за строкой состояния: надпись «Переподключение…» должна исчезнуть меньше чем через секунду, после чего без перезагрузки появится новый вывод команды.

> Heartbeat доказывает лишь то, что путь был жив мгновение назад. Он не гарантирует, что следующее сообщение дойдёт, поэтому каждое сообщение по-прежнему несёт свою ревизию, а сервер по-прежнему заполняет пропуски.

### Вспомогательная функция повтора

Цикл переподключения использует небольшую функцию, которую может переиспользовать и код загрузки:

${code}

| Случай | До | После |
| --- | --- | --- |
| Ноутбук выходит из сна | Зависание до 2 минут | Восстановление примерно за 1 с |
| Wi-Fi переключает сеть | Зависание до сбоя следующей записи | Восстановление при следующем опоздании pong |
| Перезапуск сервера | Восстановление по закрытию | Без изменений |

${wide}

- [x] Обнаруживать опоздавший pong и переподключаться
- [x] Отправлять ping, когда страница становится видимой
- [ ] Показывать в строке состояния, сколько времени страница была офлайн

---

Протокол при этом не меняется, так что старый сервер продолжает работать с новым клиентом. Если после этого изменения страница снова зависнет, строка журнала \`heartbeat late by …\` покажет, опоздал ли pong или медленным было само переподключение, — именно туда я посмотрю в первую очередь.`,

  de: `# Warum die Synchronisierung nach dem Aufwachen des Laptops hängt

Ich habe den Hänger auf den Wiederverbindungspfad in \`sync/socket.ts\` zurückgeführt. Während der Laptop schläft, hält das Betriebssystem die TCP-Verbindung formal offen, sodass der Client nie ein Schließereignis sieht; nach dem Aufwachen liegt der erste Schreibvorgang **bis zu zwei Minuten** im Kernelpuffer, bevor die Verbindung für tot erklärt wird. In dieser Zeit zeigt die Seite den zuletzt empfangenen Zustand und wirkt eingefroren, obwohl auf dem Server alles in Ordnung ist. Die Lösung: Ein verpasster Heartbeat gilt als Verbindungsabbruch, statt auf die Verbindungsabbruchsbenachrichtigung des Betriebssystems zu warten.

## Was sich ändert

Der Client sendet schon heute alle 15 Sekunden einen Ping, hat aber nie geprüft, ob das passende Pong ankommt. Jetzt prüft er es und handelt danach:

- Kommt ein Pong mehr als **5 Sekunden** zu spät, schließt der Client den Socket und startet die normale Wiederverbindung, die alles nach der letzten Revision, die die Seite gesehen hat, erneut überträgt.
- Wird die Seite wieder sichtbar (\`visibilitychange\`), sendet sie sofort einen Ping, statt auf den nächsten Takt zu warten; ein gerade aufgewachter Laptop weiß so innerhalb einer Sekunde, ob seine Verbindung die Nacht überstanden hat.
- Wiederverbindungsversuche warten exponentiell länger, beginnend bei 250 ms und begrenzt auf 30 Sekunden, damit ein Telefon im Tunnel nicht seinen Akku leert.
  - Die Wartezeit wird zurückgesetzt, sobald eine Verbindung eine volle Minute stabil bleibt.
  - Ein manuelles **Reload** überspringt die Wartezeit.

## So prüfen Sie es

1. Öffnen Sie eine Unterhaltung und starten Sie einen lang laufenden Befehl, zum Beispiel \`sleep 600 && echo done\`.
2. Versetzen Sie den Rechner mindestens fünf Minuten in den Ruhezustand und wecken Sie ihn dann.
3. Beobachten Sie die Statuszeile: Sie sollte weniger als eine Sekunde lang *Verbindung wird wiederhergestellt…* zeigen und dann ohne Neuladen die neue Ausgabe des Befehls anzeigen.

> Ein Heartbeat beweist nur, dass der Weg vor einem Augenblick noch lebte. Er beweist nicht, dass die nächste Nachricht ankommt; deshalb trägt jede Nachricht weiterhin ihre Revision, und der Server füllt weiterhin jede Lücke.

### Die Wiederholungsfunktion

Die Wiederverbindungsschleife nutzt eine kleine Hilfsfunktion, die auch der Upload-Code verwenden kann:

${code}

| Fall | Vorher | Nachher |
| --- | --- | --- |
| Laptop wacht aus dem Ruhezustand auf | Bis zu 2 min eingefroren | Erholt sich in etwa 1 s |
| WLAN wechselt das Netz | Eingefroren, bis der nächste Schreibvorgang scheitert | Erholt sich beim nächsten verspäteten Pong |
| Serverneustart | Erholt sich beim Schließen | Unverändert |

${wide}

- [x] Verspätetes Pong erkennen und neu verbinden
- [x] Ping senden, wenn die Seite sichtbar wird
- [ ] In der Statuszeile anzeigen, wie lange die Seite offline war

---

Am Protokoll ändert sich nichts, ein älterer Server arbeitet also weiter mit dem neuen Client. Hängt die Seite nach dieser Änderung erneut, verrät die Protokollzeile \`heartbeat late by …\`, ob das Pong zu spät kam oder die Wiederverbindung selbst langsam war – dort würde ich zuerst nachsehen.`,

  vi: `# Vì sao đồng bộ bị treo sau khi máy tính xách tay thức dậy

Tôi đã lần ra nguyên nhân nằm ở đường kết nối lại trong \`sync/socket.ts\`. Khi máy ngủ, hệ điều hành vẫn giữ kết nối TCP mở trên danh nghĩa, nên phía máy khách không bao giờ nhận được sự kiện đóng; khi máy thức dậy, lần ghi đầu tiên nằm trong bộ đệm của nhân **tới hai phút** rồi kết nối mới bị coi là đã chết. Trong suốt thời gian đó trang hiển thị trạng thái cuối cùng nhận được và trông như bị đơ, dù phía máy chủ không hề có vấn đề gì. Cách sửa là coi một nhịp tim bị lỡ là mất kết nối, thay vì chờ hệ điều hành báo cho chúng ta.

## Những gì thay đổi

Máy khách vốn đã gửi ping mỗi 15 giây, nhưng chưa bao giờ kiểm tra xem pong tương ứng có về hay không. Giờ thì nó kiểm tra và hành động theo kết quả:

- Pong trễ hơn **5 giây** sẽ khiến máy khách đóng socket và bắt đầu kết nối lại như bình thường, phát lại mọi thứ sau phiên bản cuối cùng mà trang đã thấy.
- Khi trang hiện lại (\`visibilitychange\`), nó gửi ping ngay thay vì chờ chu kỳ tiếp theo, nhờ vậy một chiếc máy vừa thức dậy biết được trong vòng một giây liệu kết nối còn sống hay không.
- Các lần kết nối lại giãn cách theo cấp số nhân, bắt đầu từ 250 ms và tối đa 30 giây, để điện thoại trong đường hầm không cạn pin.
  - Khoảng chờ được đặt lại khi kết nối ổn định trọn một phút.
  - Nút **Reload** thủ công bỏ qua khoảng chờ.

## Cách kiểm tra

1. Mở một cuộc trò chuyện và chạy một lệnh dài, ví dụ \`sleep 600 && echo done\`.
2. Cho máy ngủ ít nhất năm phút rồi đánh thức nó.
3. Quan sát dòng trạng thái: dòng *Đang kết nối lại…* phải biến mất trong chưa đầy một giây, sau đó kết quả mới của lệnh hiện ra mà không cần tải lại trang.

> Nhịp tim chỉ chứng minh đường truyền vừa còn sống một khoảnh khắc trước. Nó không đảm bảo tin nhắn tiếp theo sẽ tới, vì thế mỗi tin nhắn vẫn mang phiên bản của nó và máy chủ vẫn lấp đầy mọi khoảng trống.

### Hàm hỗ trợ thử lại

Vòng lặp kết nối lại dùng một hàm nhỏ mà mã tải lên cũng có thể dùng chung:

${code}

| Trường hợp | Trước | Sau |
| --- | --- | --- |
| Máy thức dậy sau khi ngủ | Đơ tới 2 phút | Phục hồi trong khoảng 1 giây |
| Wi-Fi chuyển mạng | Đơ cho đến khi lần ghi tiếp theo thất bại | Phục hồi ở lần pong trễ tiếp theo |
| Máy chủ khởi động lại | Phục hồi khi đóng | Không đổi |

${wide}

- [x] Phát hiện pong trễ và kết nối lại
- [x] Gửi ping khi trang hiện lại
- [ ] Hiển thị trên dòng trạng thái trang đã ngoại tuyến bao lâu

---

Không có thay đổi nào với giao thức, nên máy chủ cũ vẫn hoạt động với máy khách mới. Nếu sau thay đổi này trang lại bị đơ, dòng nhật ký \`heartbeat late by …\` sẽ cho biết pong bị trễ hay chính việc kết nối lại bị chậm — đó là chỗ đầu tiên tôi sẽ xem.`,
}
