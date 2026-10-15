import type { Language } from '@demicodes/web-ui/i18n/languages'
import type { TextRule } from '../components/GalleryTextRules.vue'

/**
 * How each language Demi speaks besides English is written
 * (`localization.md`): its rules, each with what follows it and what
 * breaks it, and its glossary, Demi's terms in that language. English is
 * the rest of the Writing page.
 */

/** The rule every language follows first. */
export const everyLanguageRule: TextRule = {
  title: 'VS Code’s translation first',
  rule: 'Each language takes the wording, form and tone VS Code’s translation in that language uses for the same thing (its language packs, microsoft/vscode-loc), then macOS’s, then Microsoft’s and Apple’s style guides for the language. VS Code wins where they differ. The glossaries below follow it: where a term of Demi’s is one of VS Code’s, it is VS Code’s word.',
  right: ['帐户', '提交更改', '終端機', 'コミットされていない変更', '작업 영역'],
  wrong: ['账号', '未コミットの変更', '작업 공간'],
  source: 'VS Code language packs (microsoft/vscode-loc); macOS 26; Microsoft and Apple style guides',
}

/** The rules the English page holds that hold in every language too. */
const englishRules = (right: readonly string[], wrong: readonly string[]): TextRule => ({
  title: 'The English rules hold',
  rule: 'Brevity, the most important thing first, no semicolons, and figures for numbers hold as on the rest of this page.',
  right,
  wrong,
  source: 'The Writing page’s English rules',
})

export interface LanguageWriting {
  language: Exclude<Language, 'en'>
  /** The section's title: the language's name in itself. */
  title: string
  rules: readonly TextRule[]
  /** Demi's terms: the English term, this language's, and why it is not VS Code's where it is not. */
  glossary: readonly (readonly [string, string, string?])[]
}

const SIMPLIFIED: LanguageWriting = {
  language: 'zh-Hans',
  title: '简体中文',
  rules: [
    {
      title: 'A space between Chinese and Latin letters or digits',
      rule: 'A Latin word, a name or a figure inside Chinese has a half-width space on each side.',
      right: ['打开 Finder', '3 个文件', '提交到 main 分支'],
      wrong: ['打开Finder', '3个文件', '提交到main分支'],
      source: 'VS Code’s Simplified Chinese pack; Microsoft Simplified Chinese Style Guide',
    },
    {
      title: 'No space beside full-width punctuation',
      rule: 'Full-width punctuation carries its own space: nothing goes between it and a Latin word or a figure.',
      right: ['删除“OpenAI API”？', '已复制 3 项。'],
      wrong: ['删除 “OpenAI API” ？', '已复制 3 项 。'],
      source: 'Microsoft Simplified Chinese Style Guide',
    },
    {
      title: 'Full-width punctuation and “” quotes',
      rule: 'Chinese text takes full-width commas, periods, colons, question marks and parentheses, and quotes with “”.',
      right: ['已保存，可以关闭。', '删除“OpenAI API”？'],
      wrong: ['已保存,可以关闭.', '删除"OpenAI API"?'],
      source: 'VS Code’s Simplified Chinese pack; Microsoft Simplified Chinese Style Guide',
    },
    {
      title: 'No period ending a label, a menu item or a button',
      rule: 'A label, a menu item or a button is a phrase, not a sentence.',
      right: ['新建对话', '登录', '添加到队列'],
      wrong: ['新建对话。', '登录。'],
      source: 'VS Code’s Simplified Chinese pack; macOS 26',
    },
    {
      title: 'Quantities as in English',
      rule: 'A figure against its limit and a percentage are written as in English, units included.',
      right: ['1.0 / 16 GB', '62%'],
      wrong: ['1.0 / 16 吉字节', '百分之六十二'],
      source: 'VS Code’s Simplified Chinese pack',
    },
    {
      title: 'A date or a time keeps Intl’s form',
      rule: 'A value the page formats with Intl, such as a date or a time, stays as Intl writes it, without the spaces the first rule adds.',
      right: ['10月15日', '下午6:12'],
      wrong: ['10 月 15 日', '下午 6:12'],
      source: 'Intl.DateTimeFormat for zh-Hans',
    },
    englishRules(['无法连接。请检查网络。'], ['无法连接；请检查网络。']),
  ],
  glossary: [
    ['Agent', '智能体'], ['Subagent', '子智能体'], ['Conversation', '对话'], ['Project', '项目'],
    ['Host', '主机'], ['Cloud', '云'], ['Device', '设备'], ['Runner', '运行器'], ['Relay', '中继'],
    ['Provider', '供应商'], ['Model', '模型'], ['Account', '帐户'], ['Subscription', '订阅'],
    ['Profile', '配置文件'], ['Workflow', '工作流'], ['Skill', '技能'], ['Plugin', '插件'],
    ['Credential', '凭据'], ['Instructions', '指令'], ['Context', '上下文'], ['Compaction', '压缩'],
    ['Reasoning', '推理'], ['Turn', '轮次'], ['Transcript', '对话记录'], ['Attachment', '附件'],
    ['Working directory', '工作目录'], ['Workspace', '工作区'], ['Changes', '更改'],
    ['Uncommitted Changes', '未提交的更改'], ['Commit', '提交'], ['Branch', '分支'], ['Sign in', '登录'],
    ['Passkey', '通行密钥'], ['Usage', '使用情况'], ['Limit', '限制'], ['Rotation', '轮换'],
    ['Terminal', '终端'], ['Command', '命令'], ['Archive', '存档'], ['File', '文件'], ['Folder', '文件夹'],
    ['Settings', '设置'],
  ],
}

const TRADITIONAL: LanguageWriting = {
  language: 'zh-Hant',
  title: '繁體中文',
  rules: [
    {
      title: 'Taiwan usage',
      rule: 'Words and phrasing are Taiwan’s, as VS Code’s Traditional Chinese pack writes them: 檔案, 資料夾, 設定.',
      right: ['開啟檔案', '儲存設定'],
      wrong: ['打開文件', '保存設置'],
      source: 'VS Code’s Traditional Chinese pack; Microsoft Traditional Chinese Style Guide',
    },
    {
      title: 'A space between Chinese and Latin letters or digits',
      rule: 'A Latin word, a name or a figure inside Chinese has a half-width space on each side.',
      right: ['開啟 Finder', '3 個檔案'],
      wrong: ['開啟Finder', '3個檔案'],
      source: 'VS Code’s Traditional Chinese pack',
    },
    {
      title: 'Full-width punctuation and 「」 quotes, no space beside them',
      rule: 'Chinese text takes full-width punctuation and quotes with 「」. Nothing goes between them and a Latin word or a figure.',
      right: ['刪除「OpenAI API」？', '已儲存，可以關閉。'],
      wrong: ['刪除“OpenAI API”？', '刪除 「OpenAI API」 ？', '已儲存,可以關閉.'],
      source: 'VS Code’s Traditional Chinese pack; Microsoft Traditional Chinese Style Guide',
    },
    {
      title: 'No period ending a label, a menu item or a button',
      rule: 'A label, a menu item or a button is a phrase, not a sentence.',
      right: ['新增對話', '登入'],
      wrong: ['新增對話。', '登入。'],
      source: 'VS Code’s Traditional Chinese pack; macOS 26',
    },
    {
      title: 'Quantities as in English, dates and times as Intl writes them',
      rule: 'A figure against its limit and a percentage are written as in English. A date or a time stays as Intl writes it.',
      right: ['1.0 / 16 GB', '62%', '10月15日', '下午6:12'],
      wrong: ['百分之六十二', '10 月 15 日'],
      source: 'Intl.DateTimeFormat for zh-Hant',
    },
    englishRules(['無法連線。請檢查網路。'], ['無法連線；請檢查網路。']),
  ],
  glossary: [
    ['Agent', '代理程式'], ['Subagent', '子代理程式'], ['Conversation', '對話'], ['Project', '專案'],
    ['Host', '主機'], ['Cloud', '雲端'], ['Device', '裝置'], ['Runner', '執行器'], ['Relay', '中繼'],
    ['Provider', '供應商'], ['Model', '模型'], ['Account', '帳戶'], ['Subscription', '訂閱'],
    ['Profile', '設定檔'], ['Workflow', '工作流程'], ['Skill', '技能'], ['Plugin', '外掛程式'],
    ['Credential', '憑證'], ['Instructions', '指示'], ['Context', '上下文', 'Not VS Code’s 內容, which is also Content'], ['Compaction', '壓縮'],
    ['Reasoning', '推理'], ['Turn', '輪次'], ['Transcript', '對話紀錄'], ['Attachment', '附件'],
    ['Working directory', '工作目錄'], ['Workspace', '工作區'], ['Changes', '變更'],
    ['Uncommitted Changes', '未提交的變更'], ['Commit', '提交'], ['Branch', '分支'], ['Sign in', '登入'],
    ['Passkey', '通行密鑰'], ['Usage', '使用量'], ['Limit', '限制'], ['Rotation', '輪換'],
    ['Terminal', '終端機'], ['Command', '命令'], ['Archive', '封存'], ['File', '檔案'], ['Folder', '資料夾'],
    ['Settings', '設定'],
  ],
}

const JAPANESE: LanguageWriting = {
  language: 'ja',
  title: '日本語',
  rules: [
    {
      title: 'A half-width space between Japanese and Latin letters or digits',
      rule: 'A Latin word, a name or a figure inside Japanese has a half-width space on each side, as Apple’s and VS Code’s Japanese do.',
      right: ['Finder で開く', '3 個のファイル'],
      wrong: ['Finderで開く', '3個のファイル'],
      source: 'macOS 26; VS Code’s Japanese pack',
    },
    {
      title: '「」 quotes and full-width 。、',
      rule: 'A quoted name stands between 「」. A sentence takes full-width 。 and 、.',
      right: ['「OpenAI API」を削除しますか？', '名前、パス'],
      wrong: ['"OpenAI API"を削除しますか?', '名前, パス'],
      source: 'VS Code’s Japanese pack; Microsoft Japanese Style Guide',
    },
    {
      title: 'Full-width parentheses, a half-width colon',
      rule: 'A remark in parentheses takes full-width （） without spaces. A colon before a value, a reason or a path is the half-width one followed by a space, as macOS’s labels write it.',
      right: ['デフォルト（{window}）', '移動できませんでした: {reason}'],
      wrong: ['既定 ({window})', '移動できませんでした：{reason}'],
      source: 'macOS 26; Microsoft Japanese Style Guide',
    },
    {
      title: 'Labels and buttons are nouns or short verbs, without 。',
      rule: 'A label, a menu item or a button names the thing or the action, as macOS’s do.',
      right: ['保存', 'キャンセル', 'キューに追加'],
      wrong: ['保存します。', 'キャンセルする。'],
      source: 'macOS 26; VS Code’s Japanese pack',
    },
    {
      title: 'Sentences in です・ます',
      rule: 'A sentence the page says is polite, in です・ます.',
      right: ['接続できませんでした。'],
      wrong: ['接続できなかった。'],
      source: 'Microsoft Japanese Style Guide',
    },
    {
      title: 'Quantities as in English, dates and times as Intl writes them',
      rule: 'A figure against its limit and a percentage are written as in English. A date or a time stays as Intl writes it.',
      right: ['1.0 / 16 GB', '62%', '10月15日', '18:12'],
      wrong: ['1.0 / 16 ギガバイト', '10 月 15 日'],
      source: 'Intl.DateTimeFormat for ja',
    },
    englishRules(['接続できませんでした。ネットワークを確認してください。'], ['接続できませんでした；ネットワークを確認してください。']),
  ],
  glossary: [
    ['Agent', 'エージェント', 'ChatGPT, Apple'],
    ['Subagent', 'サブエージェント', 'Codex, ChatGPT'],
    ['Conversation', 'チャット', 'Apple and ChatGPT’s sidebar say チャット for a conversation; Codex’s 会話 reads as a spoken dialogue'],
    ['New conversation', '新しいチャット', 'Codex, ChatGPT, Claude'],
    ['Project', 'プロジェクト', 'Codex, ChatGPT, Apple'],
    ['Host', 'ホスト', 'ChatGPT, Apple'],
    ['Cloud', 'クラウド', 'ChatGPT, Apple'],
    ['Device', 'デバイス', 'Apple; its 装置 is the older word'],
    ['Runner', 'ランナー', 'No product has it; the katakana of the English'],
    ['Relay', 'リレー', 'No product has it; the katakana of the English'],
    ['Provider', 'プロバイダー', 'Not Apple’s 提供元, which names a data source; a model vendor’s service is a プロバイダー'],
    ['Model', 'モデル', 'ChatGPT, Apple'],
    ['Account', 'アカウント', 'Codex, ChatGPT, Apple; not 口座, a bank account'],
    ['Add Account', 'アカウントを追加', 'Codex, Apple'],
    ['Subscription', 'サブスクリプション', 'Apple'],
    ['Plan (paid)', 'プラン', 'Codex, ChatGPT'],
    ['Profile (a subagent profile)', 'プロファイル', 'Apple’s プロファイル, a set of settings'],
    ['Profile (a person’s)', 'プロフィール', 'Codex, ChatGPT; a different thing from a settings profile'],
    ['Workflow', 'ワークフロー', 'Apple'],
    ['Skill', 'スキル', 'Codex, ChatGPT'],
    ['Plugin', 'プラグイン', 'ChatGPT, Apple'],
    ['Extension', '拡張機能', 'ChatGPT, Claude; Apple’s 機能拡張 is the system’s own kind'],
    ['Connector', 'コネクタ', 'ChatGPT'],
    ['Credential', '認証情報', 'No product names it; the everyday word for sign-in data'],
    ['API key', 'API キー', 'ChatGPT, spaced as the first rule'],
    ['Instructions', '指示', 'Codex, ChatGPT, Apple; not 手順, which is steps'],
    ['Memory', 'メモリ', 'ChatGPT, Apple'],
    ['Context', 'コンテキスト', 'ChatGPT; Apple’s コンテクスト is rarer'],
    ['Context window', 'コンテキストウィンドウ', 'The window is the model’s limit, not ChatGPT’s 画面'],
    ['Compaction', '圧縮', 'ChatGPT; not コンパクト'],
    ['Reasoning', '推論', 'ChatGPT'],
    ['Reasoning effort', '思考量', 'ChatGPT; Codex’s 負荷 reads as a machine load'],
    ['Turn', 'ターン', 'No product has it; Apple’s 変更 is another meaning'],
    ['Transcript', '会話履歴', 'Not Apple’s and Codex’s 文字起こし, which is speech to text'],
    ['Attachment', '添付ファイル', 'ChatGPT, Apple'],
    ['Working directory', '作業ディレクトリ', 'ChatGPT'],
    ['Workspace', 'ワークスペース', 'Codex, ChatGPT'],
    ['Changes', '変更', 'Codex, ChatGPT, Claude'],
    ['Uncommitted Changes', '未コミットの変更', 'ChatGPT'],
    ['Commit', 'コミット', 'Codex, ChatGPT'],
    ['Branch', 'ブランチ', 'Codex, ChatGPT'],
    ['Worktree', 'ワークツリー', 'Katakana of the Git word, as Codex and ChatGPT keep it in English'],
    ['Sign in', 'サインイン', 'ChatGPT, Apple; Codex’s ログイン is the older word'],
    ['Sign out', 'サインアウト', 'Apple, ChatGPT'],
    ['Passkey', 'パスキー', 'ChatGPT'],
    ['Usage', '使用量', 'Codex, Claude'],
    ['Usage limit', '利用上限', 'ChatGPT’s 利用制限 reads as a restriction; 上限 is the ceiling the user reaches'],
    ['Limit (usage)', '上限', 'Apple’s 使用量の制限に到達しました, shortened'],
    ['Rate limit', 'レート制限', 'ChatGPT'],
    ['Rotation', 'ローテーション', 'Not Apple’s 回転, which is turning'],
    ['Terminal', 'ターミナル', 'ChatGPT, Claude, Apple; Codex’s 端末 is a device'],
    ['Command', 'コマンド', 'ChatGPT, Apple'],
    ['Run', '実行', 'ChatGPT, Apple'],
    ['Running', '実行中', 'ChatGPT; Apple’s ランニング is a sport'],
    ['Stop', '停止', 'Codex, ChatGPT, Apple'],
    ['Resume', '再開', 'ChatGPT, Apple'],
    ['Continue', '続ける', 'Codex, ChatGPT, Claude, Apple'],
    ['Retry', '再試行', 'Codex, ChatGPT, Apple'],
    ['Queue / Queued', 'キュー / 待機中', 'ChatGPT’s 待機中, Apple’s キュー'],
    ['Steer', '今すぐ反映', 'ChatGPT; replaces 割り込み, which reads as an interruption of the agent'],
    ['Archive', 'アーカイブ', 'ChatGPT, Apple'],
    ['Archived', 'アーカイブ済み', 'The state of Archive'],
    ['Unarchive', 'アーカイブを解除', 'ChatGPT'],
    ['Pin', 'ピン留め', 'Codex, ChatGPT, Apple'],
    ['Unpin', 'ピン留めを外す', 'The everyday verb for taking a pin off; Codex, ChatGPT and Apple’s 解除 is the formal one'],
    ['Rename', '名前を変更', 'Codex, ChatGPT; Apple’s 名称変更 is a noun'],
    ['Delete', '削除', 'Codex, ChatGPT, Apple'],
    ['Share', '共有', 'Codex, ChatGPT, Apple'],
    ['Fork', '分岐', 'ChatGPT; Apple’s フォーク is a table fork'],
    ['Regenerate', '再生成', 'Not ChatGPT’s もう一度試す, which is Try again and runs the request once more'],
    ['Settings', '設定', 'Codex, ChatGPT, Claude, Apple'],
    ['General', '一般', 'ChatGPT, Apple'],
    ['Filter', 'フィルタ', 'Apple; ChatGPT’s フィルター is the longer spelling'],
    ['Allow', '許可', 'ChatGPT, Claude, Apple'],
    ['Always Allow', '常に許可', 'ChatGPT, Claude, Apple'],
    ['Deny', '拒否', 'Codex, ChatGPT, Apple'],
    ['Permissions', '権限', 'ChatGPT; not 許可, which is Allow'],
    ['Sandbox', 'サンドボックス', 'ChatGPT, Apple'],
    ['Local', 'ローカル', 'ChatGPT, Apple'],
    ['Remote', 'リモート', 'ChatGPT; Apple’s リモコン is a remote control'],
    ['Thinking', '思考中', 'ChatGPT'],
    ['Copy', 'コピー', 'Codex, ChatGPT, Claude, Apple'],
    ['Copied', 'コピーしました', 'Codex, ChatGPT, Apple'],
    ['Undo', '元に戻す', 'Codex, ChatGPT, Claude'],
    ['Open With', 'このアプリケーションで開く', 'Apple'],
    ['File', 'ファイル', 'Codex, ChatGPT, Claude, Apple'],
    ['Folder', 'フォルダ', 'Apple, Codex; ChatGPT writes フォルダー as well'],
  ],
}

const KOREAN: LanguageWriting = {
  language: 'ko',
  title: '한국어',
  rules: [
    {
      title: 'Normal Korean word spacing',
      rule: 'Words are spaced as Korean spaces them. A counter follows its figure without a space.',
      right: ['새 대화 만들기', '파일 3개'],
      wrong: ['새대화만들기', '파일 3 개'],
      source: 'VS Code’s Korean pack; Microsoft Korean Style Guide',
    },
    {
      title: 'Sentences in 합니다체',
      rule: 'A sentence the page says is formal, in 합니다체.',
      right: ['연결할 수 없습니다.'],
      wrong: ['연결할 수 없어요.'],
      source: 'VS Code’s Korean pack; Microsoft Korean Style Guide',
    },
    {
      title: 'No period ending a label, a menu item or a button',
      rule: 'A label, a menu item or a button names the thing or the action.',
      right: ['저장', '취소', '큐에 추가'],
      wrong: ['저장.', '취소합니다.'],
      source: 'macOS 26; VS Code’s Korean pack',
    },
    {
      title: 'Quantities as in English, dates and times as Intl writes them',
      rule: 'A figure against its limit and a percentage are written as in English. A date or a time stays as Intl writes it.',
      right: ['1.0 / 16 GB', '62%', '10월 15일', '오후 6:12'],
      wrong: ['1.0 / 16 기가바이트'],
      source: 'Intl.DateTimeFormat for ko',
    },
    englishRules(['연결할 수 없습니다. 네트워크를 확인하십시오.'], ['연결할 수 없습니다; 네트워크를 확인하십시오.']),
  ],
  glossary: [
    ['Agent', '에이전트'], ['Subagent', '하위 에이전트'], ['Conversation', '대화'], ['Project', '프로젝트'],
    ['Host', '호스트'], ['Cloud', '클라우드'], ['Device', '장치'], ['Runner', '러너'], ['Relay', '릴레이'],
    ['Provider', '공급자'], ['Model', '모델'], ['Account', '계정'], ['Subscription', '구독'],
    ['Profile', '프로필'], ['Workflow', '워크플로'], ['Skill', '기술'], ['Plugin', '플러그 인'],
    ['Credential', '자격 증명'], ['Instructions', '지침'], ['Context', '컨텍스트'], ['Compaction', '압축'],
    ['Reasoning', '추론'], ['Turn', '턴'], ['Transcript', '대화 기록'], ['Attachment', '첨부 파일'],
    ['Working directory', '작업 디렉터리'], ['Workspace', '작업 영역'], ['Changes', '변경 내용'],
    ['Uncommitted Changes', '커밋되지 않은 변경 내용'], ['Commit', '커밋'], ['Branch', '분기'], ['Sign in', '로그인'],
    ['Passkey', '패스키'], ['Usage', '사용량'], ['Limit', '한도'], ['Rotation', '순환'],
    ['Terminal', '터미널'], ['Command', '명령'], ['Archive', '보관'], ['File', '파일'], ['Folder', '폴더'],
    ['Settings', '설정'],
  ],
}

export const LANGUAGE_WRITING: readonly LanguageWriting[] = [SIMPLIFIED, TRADITIONAL, JAPANESE, KOREAN]
