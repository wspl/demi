import {Audio} from "@remotion/media";
import React from "react";
import {
  AbsoluteFill,
  Composition,
  Easing,
  interpolate,
  Sequence,
  staticFile,
  useCurrentFrame,
} from "remotion";

const FPS = 30;
const SCENE = 150;
const TOTAL = 1080;

const colors = {
  bg: "#07080d",
  panel: "#10131d",
  panel2: "#151928",
  text: "#f7f8ff",
  muted: "#9aa4bd",
  blue: "#65a8ff",
  violet: "#9b87ff",
  cyan: "#55e6d4",
  green: "#6ee7a7",
};

const ease = Easing.bezier(0.16, 1, 0.3, 1);

const clamp = {
  extrapolateLeft: "clamp" as const,
  extrapolateRight: "clamp" as const,
};

const Background: React.FC = () => {
  const frame = useCurrentFrame();
  return (
    <AbsoluteFill style={{backgroundColor: colors.bg, overflow: "hidden"}}>
      <div
        style={{
          position: "absolute",
          inset: 0,
          opacity: 0.22,
          backgroundImage:
            "linear-gradient(rgba(255,255,255,.045) 1px, transparent 1px), linear-gradient(90deg, rgba(255,255,255,.045) 1px, transparent 1px)",
          backgroundSize: "72px 72px",
          translate: `${interpolate(frame, [0, TOTAL], [0, -72])}px ${interpolate(frame, [0, TOTAL], [0, -36])}px`,
        }}
      />
      <div
        style={{
          position: "absolute",
          width: 900,
          height: 900,
          borderRadius: "50%",
          left: -300,
          top: -360,
          background: "radial-gradient(circle, rgba(72,120,255,.22), transparent 67%)",
          translate: `${interpolate(frame, [0, TOTAL], [0, 140])}px ${interpolate(frame, [0, TOTAL], [0, 90])}px`,
        }}
      />
      <div
        style={{
          position: "absolute",
          width: 820,
          height: 820,
          borderRadius: "50%",
          right: -250,
          bottom: -330,
          background: "radial-gradient(circle, rgba(143,90,255,.2), transparent 68%)",
          translate: `${interpolate(frame, [0, TOTAL], [0, -120])}px ${interpolate(frame, [0, TOTAL], [0, -70])}px`,
        }}
      />
      <div
        style={{
          position: "absolute",
          inset: 0,
          background: "radial-gradient(circle at center, transparent 35%, rgba(0,0,0,.55) 100%)",
        }}
      />
    </AbsoluteFill>
  );
};

const Fade: React.FC<{duration: number; children: React.ReactNode}> = ({duration, children}) => {
  const frame = useCurrentFrame();
  return (
    <AbsoluteFill
      style={{
        opacity: interpolate(frame, [0, 14, duration - 16, duration], [0, 1, 1, 0], {
          ...clamp,
          easing: ease,
        }),
      }}
    >
      {children}
    </AbsoluteFill>
  );
};

const Brand: React.FC = () => (
  <div style={{position: "absolute", left: 86, top: 62, display: "flex", alignItems: "center", gap: 14}}>
    <div
      style={{
        width: 34,
        height: 34,
        border: `3px solid ${colors.blue}`,
        borderLeftColor: colors.violet,
        rotate: "45deg",
        borderRadius: 8,
        boxShadow: "0 0 24px rgba(101,168,255,.35)",
      }}
    />
    <div style={{fontSize: 29, fontWeight: 760, letterSpacing: 7, color: colors.text}}>DEMI</div>
  </div>
);

const SceneTitle: React.FC<{eyebrow: string; title: React.ReactNode; sub: string}> = ({eyebrow, title, sub}) => {
  const frame = useCurrentFrame();
  return (
    <div style={{display: "flex", flexDirection: "column", alignItems: "center", gap: 18, textAlign: "center"}}>
      <div
        style={{
          color: colors.cyan,
          fontSize: 26,
          fontWeight: 700,
          letterSpacing: 5,
          textTransform: "uppercase",
          opacity: interpolate(frame, [4, 20], [0, 1], {...clamp, easing: ease}),
          translate: `0 ${interpolate(frame, [4, 20], [20, 0], {...clamp, easing: ease})}px`,
        }}
      >
        {eyebrow}
      </div>
      <div
        style={{
          color: colors.text,
          fontSize: 90,
          lineHeight: 0.98,
          fontWeight: 760,
          letterSpacing: -4,
          opacity: interpolate(frame, [8, 28], [0, 1], {...clamp, easing: ease}),
          translate: `0 ${interpolate(frame, [8, 28], [34, 0], {...clamp, easing: ease})}px`,
        }}
      >
        {title}
      </div>
      <div
        style={{
          color: colors.muted,
          fontSize: 40,
          lineHeight: 1.25,
          fontWeight: 430,
          opacity: interpolate(frame, [18, 38], [0, 1], {...clamp, easing: ease}),
        }}
      >
        {sub}
      </div>
    </div>
  );
};

const Intro: React.FC = () => {
  const frame = useCurrentFrame();
  return (
    <Fade duration={SCENE}>
      <AbsoluteFill style={{alignItems: "center", justifyContent: "center"}}>
        <div style={{display: "flex", flexDirection: "column", alignItems: "center", gap: 24}}>
          <div
            style={{
              width: 100,
              height: 100,
              border: `7px solid ${colors.blue}`,
              borderLeftColor: colors.violet,
              rotate: `${interpolate(frame, [0, 34], [8, 45], {...clamp, easing: ease})}deg`,
              scale: interpolate(frame, [0, 34], [0.35, 1], {...clamp, easing: ease}),
              borderRadius: 22,
              boxShadow: "0 0 80px rgba(101,168,255,.42)",
            }}
          />
          <div
            style={{
              color: colors.text,
              fontSize: 154,
              fontWeight: 780,
              letterSpacing: interpolate(frame, [10, 48], [38, 16], {...clamp, easing: ease}),
              marginLeft: 16,
              opacity: interpolate(frame, [8, 30], [0, 1], {...clamp, easing: ease}),
            }}
          >
            DEMI
          </div>
          <div
            style={{
              fontSize: 47,
              color: colors.muted,
              opacity: interpolate(frame, [32, 56], [0, 1], {...clamp, easing: ease}),
              translate: `0 ${interpolate(frame, [32, 56], [20, 0], {...clamp, easing: ease})}px`,
            }}
          >
            Build agents. <span style={{color: colors.text}}>Your architecture.</span>
          </div>
          <div
            style={{
              marginTop: 18,
              height: 3,
              width: interpolate(frame, [48, 86], [0, 470], {...clamp, easing: ease}),
              background: `linear-gradient(90deg, transparent, ${colors.blue}, ${colors.violet}, transparent)`,
            }}
          />
        </div>
      </AbsoluteFill>
    </Fade>
  );
};

const ProviderScene: React.FC = () => {
  const frame = useCurrentFrame();
  const providers = ["Claude Code", "Codex", "Anthropic", "OpenAI", "Your own"];
  return (
    <Fade duration={SCENE}>
      <Brand />
      <AbsoluteFill style={{padding: "145px 86px 95px", alignItems: "center", justifyContent: "space-between"}}>
        <SceneTitle eyebrow="provider-agnostic" title={<>One runtime. <span style={{color: colors.blue}}>Any model.</span></>} sub="A single inference contract across providers." />
        <div style={{display: "flex", alignItems: "center", justifyContent: "center", gap: 22, width: "100%"}}>
          {providers.map((provider, index) => (
            <React.Fragment key={provider}>
              <div
                style={{
                  padding: "25px 30px",
                  minWidth: 210,
                  textAlign: "center",
                  color: index === 4 ? colors.cyan : colors.text,
                  fontSize: 28,
                  fontWeight: 650,
                  borderRadius: 18,
                  border: `1px solid ${index === 4 ? "rgba(85,230,212,.55)" : "rgba(255,255,255,.13)"}`,
                  background: index === 4 ? "rgba(85,230,212,.08)" : "rgba(16,19,29,.88)",
                  opacity: interpolate(frame, [42 + index * 7, 60 + index * 7], [0, 1], {...clamp, easing: ease}),
                  translate: `0 ${interpolate(frame, [42 + index * 7, 60 + index * 7], [24, 0], {...clamp, easing: ease})}px`,
                }}
              >
                {provider}
              </div>
              {index < providers.length - 1 ? <div style={{height: 2, width: 28, background: "rgba(101,168,255,.4)"}} /> : null}
            </React.Fragment>
          ))}
        </div>
      </AbsoluteFill>
    </Fade>
  );
};

const ShellScene: React.FC = () => {
  const frame = useCurrentFrame();
  const rows = [
    ["$", "shell_exec", "run long-lived processes"],
    ["→", "shell_status", "inspect without blocking"],
    ["→", "shell_write", "stay in control"],
    ["✓", "yield", "wake exactly when needed"],
  ];
  return (
    <Fade duration={180}>
      <Brand />
      <AbsoluteFill style={{padding: "150px 100px 92px", flexDirection: "row", gap: 90, alignItems: "center"}}>
        <div style={{flex: 1, display: "flex", flexDirection: "column", gap: 24}}>
          <div style={{color: colors.cyan, fontSize: 26, fontWeight: 700, letterSpacing: 5}}>SANDBOXABLE SHELL</div>
          <div style={{fontSize: 92, fontWeight: 760, lineHeight: 0.98, letterSpacing: -4, color: colors.text}}>
            Control that<br/><span style={{color: colors.violet}}>never blocks.</span>
          </div>
          <div style={{fontSize: 39, lineHeight: 1.3, color: colors.muted}}>Long-running tools. Budgeted output. Delayed wakeups.</div>
        </div>
        <div
          style={{
            width: 790,
            borderRadius: 28,
            overflow: "hidden",
            border: "1px solid rgba(255,255,255,.14)",
            background: "rgba(11,13,20,.94)",
            boxShadow: "0 36px 100px rgba(0,0,0,.5), 0 0 90px rgba(101,168,255,.09)",
            scale: interpolate(frame, [10, 34], [0.92, 1], {...clamp, easing: ease}),
            opacity: interpolate(frame, [10, 28], [0, 1], {...clamp, easing: ease}),
          }}
        >
          <div style={{height: 66, background: colors.panel2, display: "flex", alignItems: "center", padding: "0 24px", gap: 12}}>
            {["#ff6b6b", "#ffd166", "#6ee7a7"].map((c) => <div key={c} style={{width: 14, height: 14, borderRadius: "50%", background: c}} />)}
            <div style={{marginLeft: 18, color: colors.muted, fontSize: 22}}>demi / agent-shell</div>
          </div>
          <div style={{padding: "34px 38px", display: "flex", flexDirection: "column", gap: 25}}>
            {rows.map(([symbol, name, desc], index) => (
              <div
                key={name}
                style={{
                  display: "grid",
                  gridTemplateColumns: "34px 210px 1fr",
                  alignItems: "center",
                  opacity: interpolate(frame, [38 + index * 16, 52 + index * 16], [0, 1], {...clamp, easing: ease}),
                  translate: `${interpolate(frame, [38 + index * 16, 52 + index * 16], [-18, 0], {...clamp, easing: ease})}px 0`,
                }}
              >
                <span style={{color: index === 3 ? colors.green : colors.blue, fontSize: 26}}>{symbol}</span>
                <span style={{color: colors.text, fontSize: 27, fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace"}}>{name}</span>
                <span style={{color: colors.muted, fontSize: 25}}>{desc}</span>
              </div>
            ))}
          </div>
        </div>
      </AbsoluteFill>
    </Fade>
  );
};

const HostScene: React.FC = () => {
  const frame = useCurrentFrame();
  const hosts = [
    ["LOCAL", "Node reference"],
    ["REMOTE", "Your backend"],
    ["CONTAINER", "Isolated runtime"],
    ["SANDBOX", "Policy first"],
  ];
  return (
    <Fade duration={SCENE}>
      <Brand />
      <AbsoluteFill style={{padding: "140px 86px 86px", alignItems: "center", justifyContent: "space-between"}}>
        <SceneTitle eyebrow="host-abstracted" title={<>Run <span style={{color: colors.cyan}}>anywhere.</span></>} sub="Filesystem, process and store — behind one Host contract." />
        <div style={{display: "flex", alignItems: "center", gap: 22}}>
          {hosts.map(([name, desc], index) => (
            <div
              key={name}
              style={{
                width: 350,
                height: 180,
                borderRadius: 24,
                border: "1px solid rgba(255,255,255,.13)",
                background: "linear-gradient(145deg, rgba(21,25,40,.96), rgba(10,12,20,.92))",
                display: "flex",
                flexDirection: "column",
                alignItems: "center",
                justifyContent: "center",
                gap: 16,
                opacity: interpolate(frame, [48 + index * 10, 66 + index * 10], [0, 1], {...clamp, easing: ease}),
                translate: `0 ${interpolate(frame, [48 + index * 10, 66 + index * 10], [34, 0], {...clamp, easing: ease})}px`,
                boxShadow: index === 0 ? "0 0 55px rgba(101,168,255,.12)" : "none",
              }}
            >
              <div style={{fontSize: 32, fontWeight: 750, letterSpacing: 3, color: index === 0 ? colors.blue : colors.text}}>{name}</div>
              <div style={{fontSize: 25, color: colors.muted}}>{desc}</div>
            </div>
          ))}
        </div>
      </AbsoluteFill>
    </Fade>
  );
};

const ProtocolScene: React.FC = () => {
  const frame = useCurrentFrame();
  const transports = ["IN-PROCESS", "STDIO", "WEBSOCKET"];
  const surfaces = ["YOUR APP", "REPL", "WEB UI"];
  return (
    <Fade duration={SCENE}>
      <Brand />
      <AbsoluteFill style={{padding: "138px 86px 86px", alignItems: "center", gap: 54}}>
        <SceneTitle eyebrow="transport-neutral" title={<>One protocol. <span style={{color: colors.violet}}>Every surface.</span></>} sub="The same AgentClient drives them all." />
        <div style={{display: "grid", gridTemplateColumns: "1fr 420px 1fr", alignItems: "center", gap: 38, width: "100%"}}>
          <div style={{display: "flex", justifyContent: "flex-end", gap: 16}}>
            {transports.map((label, index) => (
              <div key={label} style={{padding: "22px 20px", borderRadius: 16, background: colors.panel, border: "1px solid rgba(255,255,255,.12)", color: colors.muted, fontSize: 23, fontWeight: 650, opacity: interpolate(frame, [42 + index * 7, 58 + index * 7], [0, 1], clamp)}}>{label}</div>
            ))}
          </div>
          <div
            style={{
              height: 116,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              borderRadius: 28,
              background: "linear-gradient(110deg, rgba(101,168,255,.22), rgba(155,135,255,.22))",
              border: "1px solid rgba(136,157,255,.5)",
              color: colors.text,
              fontSize: 38,
              fontWeight: 760,
              boxShadow: "0 0 80px rgba(101,168,255,.18)",
              scale: interpolate(frame, [30, 58], [0.8, 1], {...clamp, easing: ease}),
            }}
          >
            AgentClient
          </div>
          <div style={{display: "flex", gap: 16}}>
            {surfaces.map((label, index) => (
              <div key={label} style={{padding: "22px 24px", borderRadius: 16, background: colors.panel, border: "1px solid rgba(255,255,255,.12)", color: index === 0 ? colors.cyan : colors.text, fontSize: 24, fontWeight: 700, opacity: interpolate(frame, [66 + index * 7, 82 + index * 7], [0, 1], clamp)}}>{label}</div>
            ))}
          </div>
        </div>
      </AbsoluteFill>
    </Fade>
  );
};

const PackagesScene: React.FC = () => {
  const frame = useCurrentFrame();
  const layers = [
    ["YOUR PRODUCT", "#55e6d4"],
    ["coding-agent  ·  agent", "#65a8ff"],
    ["provider  ·  shell  ·  host", "#9b87ff"],
    ["core  ·  utils", "#6f7892"],
  ];
  return (
    <Fade duration={SCENE}>
      <Brand />
      <AbsoluteFill style={{padding: "145px 100px 88px", flexDirection: "row", alignItems: "center", gap: 120}}>
        <div style={{flex: 1, display: "flex", flexDirection: "column", gap: 24}}>
          <div style={{color: colors.cyan, fontSize: 26, fontWeight: 700, letterSpacing: 5}}>COMPOSABLE BY DESIGN</div>
          <div style={{fontSize: 96, lineHeight: 0.96, fontWeight: 760, letterSpacing: -4, color: colors.text}}>Use what you need.<br/><span style={{color: colors.blue}}>Own the rest.</span></div>
          <div style={{fontSize: 38, lineHeight: 1.3, color: colors.muted}}>Strict package boundaries. Clear extension points. TypeScript end to end.</div>
        </div>
        <div style={{width: 720, display: "flex", flexDirection: "column", alignItems: "center", gap: 16}}>
          {layers.map(([label, accent], index) => (
            <div
              key={label}
              style={{
                width: 680 - index * 55,
                height: 102,
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                borderRadius: 22,
                background: `linear-gradient(90deg, ${accent}18, rgba(15,18,28,.96))`,
                border: `1px solid ${accent}70`,
                color: index === 0 ? colors.text : colors.muted,
                fontSize: index === 0 ? 31 : 28,
                fontWeight: 700,
                letterSpacing: index === 0 ? 3 : 1,
                opacity: interpolate(frame, [38 + index * 12, 56 + index * 12], [0, 1], {...clamp, easing: ease}),
                translate: `0 ${interpolate(frame, [38 + index * 12, 56 + index * 12], [28, 0], {...clamp, easing: ease})}px`,
              }}
            >
              {label}
            </div>
          ))}
        </div>
      </AbsoluteFill>
    </Fade>
  );
};

const Outro: React.FC = () => {
  const frame = useCurrentFrame();
  return (
    <Fade duration={SCENE}>
      <AbsoluteFill style={{alignItems: "center", justifyContent: "center"}}>
        <div style={{display: "flex", flexDirection: "column", alignItems: "center", textAlign: "center", gap: 26}}>
          <div
            style={{
              width: 82,
              height: 82,
              border: `6px solid ${colors.blue}`,
              borderLeftColor: colors.violet,
              rotate: "45deg",
              borderRadius: 18,
              boxShadow: "0 0 70px rgba(101,168,255,.42)",
              scale: interpolate(frame, [8, 34], [0.45, 1], {...clamp, easing: ease}),
            }}
          />
          <div style={{fontSize: 108, lineHeight: 1, fontWeight: 780, letterSpacing: -5, color: colors.text}}>Compose the agent<br/><span style={{background: `linear-gradient(90deg, ${colors.blue}, ${colors.violet}, ${colors.cyan})`, backgroundClip: "text", color: "transparent"}}>you actually need.</span></div>
          <div style={{fontSize: 40, color: colors.muted, opacity: interpolate(frame, [34, 58], [0, 1], {...clamp, easing: ease})}}>github.com/wspl/demi</div>
          <div style={{marginTop: 8, display: "flex", gap: 18, opacity: interpolate(frame, [52, 72], [0, 1], clamp)}}>
            {["TypeScript", "Apache-2.0", "Pre-1.0"].map((label) => <div key={label} style={{padding: "15px 25px", borderRadius: 999, border: "1px solid rgba(255,255,255,.15)", background: "rgba(15,18,28,.75)", color: colors.muted, fontSize: 24}}>{label}</div>)}
          </div>
        </div>
      </AbsoluteFill>
    </Fade>
  );
};

const Swoosh: React.FC = () => <Audio src={staticFile("whoosh.wav")} volume={0.22} />;

const DemiPromo: React.FC = () => (
  <AbsoluteFill style={{fontFamily: "Inter, ui-sans-serif, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"}}>
    <Background />
    <Audio
      src={staticFile("ambient.m4a")}
      volume={(frame) => interpolate(frame, [0, 60, TOTAL - 90, TOTAL], [0, 0.75, 0.75, 0], clamp)}
    />
    {[150, 300, 480, 630, 780, 930].map((from) => (
      <Sequence key={from} from={from} durationInFrames={24} layout="none"><Swoosh /></Sequence>
    ))}
    <Sequence from={1010} durationInFrames={70} layout="none"><Audio src={staticFile("ding.wav")} volume={0.12} /></Sequence>
    <Sequence name="01 — Intro" durationInFrames={SCENE}><Intro /></Sequence>
    <Sequence name="02 — Providers" from={150} durationInFrames={SCENE}><ProviderScene /></Sequence>
    <Sequence name="03 — Shell" from={300} durationInFrames={180}><ShellScene /></Sequence>
    <Sequence name="04 — Hosts" from={480} durationInFrames={SCENE}><HostScene /></Sequence>
    <Sequence name="05 — Protocol" from={630} durationInFrames={SCENE}><ProtocolScene /></Sequence>
    <Sequence name="06 — Packages" from={780} durationInFrames={SCENE}><PackagesScene /></Sequence>
    <Sequence name="07 — CTA" from={930} durationInFrames={SCENE}><Outro /></Sequence>
  </AbsoluteFill>
);

export const MyComposition = () => (
  <Composition
    id="DemiPromo"
    component={DemiPromo}
    durationInFrames={TOTAL}
    fps={FPS}
    width={1920}
    height={1080}
  />
);
