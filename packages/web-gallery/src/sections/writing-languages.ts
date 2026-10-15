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
      rule: 'Words and phrasing are Taiwan’s, as ChatGPT’s and macOS’s Traditional Chinese write them: 檔案, 資料夾, 設定, 影片, 選單.',
      right: ['開啟檔案', '儲存設定', '播放影片'],
      wrong: ['打開文件', '保存設置', '播放視頻'],
      source: 'ChatGPT and Codex (zh-TW); macOS 26',
    },
    {
      title: 'Everyday words over a vendor’s house terms',
      rule: 'Where Microsoft’s Traditional Chinese has a house term and the apps people use every day say it another way, take the everyday word. 你 is the pronoun, as in ChatGPT, Claude and macOS, not 您.',
      right: ['選單', '對話框', '分頁', '儲存庫', '你的帳號'],
      wrong: ['功能表', '對話方塊', '索引標籤', '存放庫', '您的帳戶'],
      source: 'ChatGPT (選單 45 times against 2, 儲存庫 39 against 2, 你 2,237 against 333 for 您), Codex, macOS. 對話框 is a judgement: ChatGPT writes both, 對話框 8 times and 對話方塊 21',
    },
    {
      title: 'A space between Chinese and Latin letters or digits',
      rule: 'A Latin word, a name or a figure inside Chinese has a half-width space on each side.',
      right: ['開啟 Finder', '3 個檔案'],
      wrong: ['開啟Finder', '3個檔案'],
      source: 'ChatGPT; Microsoft Traditional Chinese Style Guide',
    },
    {
      title: 'Full-width punctuation and 「」 quotes, no space beside them',
      rule: 'Chinese text takes full-width punctuation, a colon included, and quotes with 「」. Nothing goes between them and a Latin word or a figure.',
      right: ['刪除「OpenAI API」？', '已儲存，可以關閉。', '衝突：兩者皆新增'],
      wrong: ['刪除“OpenAI API”？', '刪除 「OpenAI API」 ？', '已儲存,可以關閉.', '衝突: 兩者皆新增'],
      source: 'ChatGPT and Codex (zh-TW); macOS 26',
    },
    {
      title: 'No period ending a label, a menu item or a button',
      rule: 'A label, a menu item or a button is a phrase, not a sentence.',
      right: ['新對話', '登入'],
      wrong: ['新對話。', '登入。'],
      source: 'ChatGPT; macOS 26',
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
    ['Agent', '智慧體', 'ChatGPT, Codex. Apple’s 代理程式 is its word for a background process'],
    ['Subagent', '子智慧體', 'ChatGPT, Codex (which also has 子代理)'],
    ['Conversation', '對話', 'Codex, ChatGPT, Apple'],
    ['New conversation', '新對話', 'Codex, ChatGPT. Claude’s 新增對話 is the verb form'],
    ['Project', '專案', 'Codex, ChatGPT. Apple’s 計畫案 is a different thing'],
    ['Host', '主機', 'ChatGPT, Apple'],
    ['Cloud', '雲端', 'ChatGPT, Apple'],
    ['Device', '裝置', 'Apple'],
    ['Runner', '執行器', 'Demi’s own word; none of the products has one'],
    ['Relay', '中繼', 'Demi’s own word; none of the products has one'],
    ['Provider', '供應商', 'ChatGPT, Codex. Apple’s 提供者 is a content source, a different thing'],
    ['Model', '模型', 'ChatGPT. Apple’s 機型 is a device model'],
    ['Account', '帳號', 'Apple. ChatGPT and Codex say 帳戶, which reads as a bank account; 帳號 is the everyday word for a login'],
    ['Add Account', '新增帳號', 'Codex, ChatGPT (新增) with Apple’s 帳號'],
    ['Subscription', '訂閱', 'Apple (訂閱項目 for one item)'],
    ['Plan (paid)', '方案', 'Codex, ChatGPT. Apple’s 規劃 is planning'],
    ['Profile (a subagent profile)', '設定檔', 'The everyday word for a saved set of settings. Apple’s 描述檔 is its configuration profile'],
    ['Profile (a person’s)', '個人檔案', 'Codex, ChatGPT, Apple'],
    ['Workflow', '工作流程', 'Apple'],
    ['Skill', '技能', 'Codex, ChatGPT'],
    ['Plugin', '外掛程式', 'ChatGPT. Apple’s 外掛模組 is stiffer'],
    ['Extension', '擴充功能', 'ChatGPT, Claude. Apple’s 延伸功能 is its own word'],
    ['Connector', '連接器', 'ChatGPT'],
    ['Credential', '憑證', 'No product’s interface has one; the common Taiwanese word'],
    ['API key', 'API 金鑰', 'ChatGPT'],
    ['Instructions', '指示', 'Codex, ChatGPT, Apple'],
    ['Memory', '記憶', 'ChatGPT. Apple’s 記憶體 is RAM'],
    ['Context', '上下文', 'ChatGPT. Apple’s 內容 is Content'],
    ['Context window', '上下文視窗', 'ChatGPT'],
    ['Compaction', '壓縮', 'No product has the word; ChatGPT’s 精簡 is a density setting'],
    ['Reasoning', '推理', 'Codex, ChatGPT'],
    ['Reasoning effort', '推理強度', 'Codex, ChatGPT'],
    ['Turn', '輪次', 'No product has the word; 輪次 is the common one'],
    ['Transcript', '對話紀錄', 'Codex and Apple use 逐字稿 for a speech transcript, a different thing'],
    ['Attachment', '附件', 'ChatGPT, Apple'],
    ['Working directory', '工作目錄', 'ChatGPT'],
    ['Workspace', '工作區', 'Codex, ChatGPT'],
    ['Changes', '變更', 'Codex, ChatGPT, Claude. Apple’s 變化 and 更動 are other things'],
    ['Uncommitted Changes', '未提交的變更', 'ChatGPT'],
    ['Commit', '提交', 'Codex, ChatGPT. 送交 is Microsoft’s'],
    ['Branch', '分支', 'Codex, ChatGPT'],
    ['Worktree', '工作樹', 'Codex, ChatGPT'],
    ['Sign in', '登入', 'Codex, ChatGPT, Apple'],
    ['Sign out', '登出', 'Codex, ChatGPT, Apple'],
    ['Passkey', '通行密鑰', 'ChatGPT'],
    ['Usage', '用量', 'Claude, Apple. ChatGPT’s 使用情況 is wordier'],
    ['Usage limit', '使用上限', 'ChatGPT; Apple’s 已達上限 says the same'],
    ['Limit (usage)', '上限', 'Apple'],
    ['Rate limit', '速率限制', 'ChatGPT'],
    ['Rotation', '輪換', 'Apple’s 旋轉 turns an image; 輪換 takes turns'],
    ['Terminal', '終端機', 'Codex, ChatGPT, Claude, Apple'],
    ['Command', '指令', 'ChatGPT, Apple. 命令 is Microsoft’s, kept only in 命令列 (Apple) for a command line'],
    ['Run', '執行', 'ChatGPT, Apple'],
    ['Running', '執行中', 'ChatGPT. Apple’s 跑步 is a workout'],
    ['Stop', '停止', 'Codex, ChatGPT, Apple'],
    ['Resume', '恢復', 'ChatGPT and Apple use 繼續 for both; Demi shows both together, so Resume (after a stop) is 恢復'],
    ['Continue', '繼續', 'Codex, ChatGPT, Apple'],
    ['Retry', '重試', 'Codex, ChatGPT. Apple’s 再試一次 is Try again'],
    ['Queue / Queued', '佇列 / 佇列中', 'ChatGPT, Apple'],
    ['Steer', '引導', 'ChatGPT'],
    ['Archive', '封存', 'ChatGPT, Apple'],
    ['Archived', '已封存', 'ChatGPT, Apple'],
    ['Unarchive', '取消封存', 'ChatGPT'],
    ['Pin', '釘選', 'Codex, ChatGPT, Apple'],
    ['Unpin', '取消釘選', 'Codex, ChatGPT, Apple'],
    ['Rename', '重新命名', 'Codex, ChatGPT, Apple'],
    ['Delete', '刪除', 'Codex, ChatGPT, Claude, Apple'],
    ['Share', '分享', 'Codex, ChatGPT, Apple. 共享 is sharing with a group'],
    ['Fork', '分岔', 'ChatGPT says 分支, which is a git branch here; Apple’s 叉子 is a utensil'],
    ['Regenerate', '重新產生', 'ChatGPT and Codex say 再試一次; Demi has Retry apart, so the reply is made again'],
    ['Settings', '設定', 'Codex, ChatGPT, Claude, Apple'],
    ['General', '一般', 'ChatGPT, Apple'],
    ['Filter', '篩選', 'ChatGPT. Apple’s 過濾 is for a filter that removes'],
    ['Allow', '允許', 'ChatGPT, Apple. Codex’s 核准 is Approve'],
    ['Always Allow', '一律允許', 'ChatGPT. Apple’s 永遠允許 is stiffer'],
    ['Deny', '拒絕', 'Codex, ChatGPT, Apple'],
    ['Permissions', '權限', 'ChatGPT'],
    ['Sandbox', '沙盒', 'ChatGPT'],
    ['Local', '本機', 'ChatGPT, Apple'],
    ['Remote', '遠端', 'ChatGPT, Apple. Apple’s 遙控器 is a remote control'],
    ['Thinking', '思考中', 'ChatGPT'],
    ['Copy', '複製', 'Codex, ChatGPT, Claude. Apple’s 拷貝 is its house word'],
    ['Copied', '已複製', 'Codex, ChatGPT'],
    ['Undo', '復原', 'Codex, ChatGPT, Claude. Apple’s 還原 is Restore here'],
    ['Open With', '打開方式', 'Apple. ChatGPT says 開啟方式, and Demi’s Open is 開啟 throughout'],
    ['File', '檔案', 'Codex, ChatGPT, Claude, Apple'],
    ['Folder', '資料夾', 'Codex, ChatGPT. Apple’s 檔案夾 is less usual'],
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
    ['Agent', 'エージェント'], ['Subagent', 'サブエージェント'], ['Conversation', '会話'], ['Project', 'プロジェクト'],
    ['Host', 'ホスト'], ['Cloud', 'クラウド'], ['Device', 'デバイス'], ['Runner', 'ランナー'], ['Relay', 'リレー'],
    ['Provider', 'プロバイダー'], ['Model', 'モデル'], ['Account', 'アカウント'], ['Subscription', 'サブスクリプション'],
    ['Profile', 'プロファイル'], ['Workflow', 'ワークフロー'], ['Skill', 'スキル'], ['Plugin', 'プラグイン'],
    ['Credential', '認証情報'], ['Instructions', '指示'], ['Context', 'コンテキスト'], ['Compaction', '圧縮'],
    ['Reasoning', '推論'], ['Turn', 'ターン'], ['Steer', '割り込み', 'Not VS Code’s 操作, which reads as any operation'], ['Transcript', '会話履歴'], ['Attachment', '添付ファイル'],
    ['Working directory', '作業ディレクトリ'], ['Workspace', 'ワークスペース'], ['Changes', '変更点'],
    ['Uncommitted Changes', 'コミットされていない変更'], ['Commit', 'コミット'], ['Branch', 'ブランチ'],
    ['Sign in', 'サインイン'], ['Passkey', 'パスキー'], ['Usage', '使用状況'], ['Limit', '制限'],
    ['Rotation', 'ローテーション'], ['Terminal', 'ターミナル'], ['Command', 'コマンド'], ['Archive', 'アーカイブ'],
    ['File', 'ファイル'], ['Folder', 'フォルダー'], ['Settings', '設定'],
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
