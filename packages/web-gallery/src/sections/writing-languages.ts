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
      right: ['新对话', '登录', '添加到队列'],
      wrong: ['新对话。', '登录。'],
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
    ['Agent', '智能体', 'ChatGPT, Codex; Apple’s 代理 is a proxy'],
    ['Subagent', '子智能体', 'ChatGPT, Codex'],
    ['Conversation', '对话', 'Codex, ChatGPT, Apple; ChatGPT’s 聊天 is its word for a chat'],
    ['New conversation', '新对话', 'Claude says 新建对话; ChatGPT’s 新聊天 follows the same short form'],
    ['Project', '项目', 'Codex, ChatGPT, Apple'],
    ['Host', '主机', 'ChatGPT, Apple'],
    ['Cloud', '云端', 'ChatGPT; Apple’s 云服务 is a service and its 云 alone is clipped'],
    ['Device', '设备', 'Apple, ChatGPT'],
    ['Runner', '运行器', 'No product has the word; it is what runs commands, from 运行'],
    ['Relay', '中继', 'No product has the word; 中继 is the network term'],
    ['Provider', '供应商', 'Codex and ChatGPT say 提供商, Apple 提供者, both “one who provides”; 供应商 is who sells a model, and VS Code’s 提供程序 is a code concept, not a vendor'],
    ['Model', '模型', 'ChatGPT; Apple’s 型号 is a device model'],
    ['Account', '账户', 'Codex, ChatGPT, Apple; the everyday 账户 over 帐户, and 账号 is a login name'],
    ['Add Account', '添加账户', 'Codex, Apple'],
    ['Subscription', '订阅', 'Apple'],
    ['Plan (paid)', '套餐', 'ChatGPT, for a paid plan; its 计划 is a to-do'],
    ['Profile (a subagent profile)', '配置', 'A set of settings; Apple’s 描述文件 is its configuration profile'],
    ['Profile (a person’s)', '个人资料', 'Codex, ChatGPT, Apple'],
    ['Workflow', '工作流', 'ChatGPT, for GitHub’s workflow; Apple’s 工作流程 is longer'],
    ['Skill', '技能', 'ChatGPT, Codex'],
    ['Plugin', '插件', 'ChatGPT, Apple'],
    ['Extension', '扩展', 'Claude, Apple; ChatGPT’s 扩展程序 is the browser kind'],
    ['Connector', '连接器', 'ChatGPT'],
    ['Credential', '凭据', 'No app has the word; 凭据 is the usual term, as in VS Code'],
    ['API key', 'API 密钥', 'ChatGPT'],
    ['Instructions', '指令', 'ChatGPT’s 自定义指令; Codex and Apple’s 说明 is a manual'],
    ['Memory', '记忆', 'ChatGPT’s product feature; Apple’s 内存 is RAM'],
    ['Context', '上下文', 'ChatGPT; Apple’s 情境 is a Focus mode'],
    ['Context window', '上下文窗口', 'ChatGPT, Codex'],
    ['Compaction', '压缩', 'ChatGPT; 紧凑 is a density'],
    ['Reasoning', '推理', 'ChatGPT, Codex'],
    ['Reasoning effort', '推理强度', 'Codex, ChatGPT'],
    ['Turn', '轮次', 'No app has a word for it; ChatGPT’s 一轮 is a count, 轮次 the noun'],
    ['Transcript', '对话记录', 'Codex’s 转录文本 is a meeting’s; this is a record of the conversation'],
    ['Attachment', '附件', 'Apple, ChatGPT'],
    ['Working directory', '工作目录', 'ChatGPT'],
    ['Workspace', '工作空间', 'Codex, ChatGPT; VS Code’s 工作区 is its own workspace'],
    ['Changes', '更改', 'Claude, ChatGPT, Codex, as in VS Code; 变更 is formal and Apple’s 变化 is a difference'],
    ['Uncommitted Changes', '未提交的更改', 'ChatGPT'],
    ['Commit', '提交', 'Codex, ChatGPT'],
    ['Branch', '分支', 'Codex, ChatGPT'],
    ['Worktree', '工作树', 'Codex, ChatGPT'],
    ['Sign in', '登录', 'Codex, ChatGPT, Apple'],
    ['Sign out', '退出登录', 'Codex, ChatGPT, Apple; 注销 reads as deleting an account'],
    ['Passkey', '通行密钥', 'ChatGPT'],
    ['Usage', '用量', 'Claude, ChatGPT; 使用情况 is wordy as a label'],
    ['Usage limit', '用量限额', 'ChatGPT’s 使用限额, with Claude’s 用量'],
    ['Limit (usage)', '限额', 'ChatGPT; 限制 is a restriction'],
    ['Rate limit', '速率限制', 'ChatGPT’s 速率限制; its 额度 is a quota'],
    ['Rotation', '轮换', 'Apple’s 旋转 turns a picture'],
    ['Terminal', '终端', 'Codex, ChatGPT, Claude, Apple'],
    ['Command', '命令', 'ChatGPT, Apple'],
    ['Run', '运行', 'ChatGPT, Apple'],
    ['Running', '正在运行', 'ChatGPT'],
    ['Stop', '停止', 'Codex, ChatGPT, Apple'],
    ['Resume', '恢复', 'ChatGPT, one of its two words; 继续 stays for Continue'],
    ['Continue', '继续', 'Codex, ChatGPT, Apple'],
    ['Retry', '重试', 'Codex, ChatGPT, Apple'],
    ['Queue / Queued', '队列 / 已排队', 'Apple; ChatGPT’s 排队'],
    ['Steer', '引导', 'ChatGPT'],
    ['Archive', '归档', 'ChatGPT, Apple; the everyday word over 存档'],
    ['Archived', '已归档', 'As Archive'],
    ['Unarchive', '取消归档', 'ChatGPT'],
    ['Restore', '恢复', 'Apple, ChatGPT; the button on an archived conversation, where the English says Restore'],
    ['Pin', '置顶', 'Codex, ChatGPT, Apple'],
    ['Unpin', '取消置顶', 'Codex, ChatGPT, Apple'],
    ['Remove', '移除', 'Apple, ChatGPT; Delete is 删除, for what is destroyed'],
    ['Rename', '重命名', 'Codex, ChatGPT; Apple’s 重新命名 is the longer form'],
    ['Delete', '删除', 'Codex, ChatGPT, Apple'],
    ['Share', '分享', 'Codex, ChatGPT, Apple, which also says 共享'],
    ['Fork', '分叉', 'ChatGPT; 分支 is the git word'],
    ['Regenerate', '重新生成', 'Plain words; ChatGPT’s 重试 is its Try again, which is Retry here'],
    ['Settings', '设置', 'Codex, ChatGPT, Claude, Apple'],
    ['General', '通用', 'Apple; ChatGPT uses both 通用 and 常规'],
    ['Filter', '筛选', 'ChatGPT; Apple’s 过滤 is for removing'],
    ['Allow', '允许', 'ChatGPT, Apple; Codex’s 批准 is stiffer'],
    ['Always Allow', '始终允许', 'ChatGPT, Apple'],
    ['Deny', '拒绝', 'Codex, ChatGPT, Apple'],
    ['Permissions', '权限', 'ChatGPT'],
    ['Sandbox', '沙盒', 'ChatGPT, Apple'],
    ['Local', '本地', 'ChatGPT, Apple'],
    ['Remote', '远程', 'ChatGPT, Apple'],
    ['Thinking', '正在思考', 'ChatGPT'],
    ['Copy', '复制', 'Codex, ChatGPT, Claude; Apple’s 拷贝 is its own Mac word'],
    ['Copied', '已复制', 'Codex, ChatGPT'],
    ['Undo', '撤销', 'Codex, ChatGPT, Claude, Apple; 撤消 is the old form'],
    ['Open With', '打开方式', 'Codex, ChatGPT, Apple'],
    ['File', '文件', 'Codex, ChatGPT, Claude, Apple'],
    ['Folder', '文件夹', 'Codex, ChatGPT, Apple'],
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
