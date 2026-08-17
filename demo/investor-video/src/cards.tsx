import type { FC } from "react";
import { AbsoluteFill } from "remotion";
import { colors } from "./theme";
import { TypeLine, display, mono } from "./type";

export const TitleCard: FC<{
  kicker: string;
  line1: string;
  line2: string;
  accent?: string;
}> = ({ kicker, line1, line2, accent = colors.rate }) => {
  const line2Delay = line1.length + 8;
  return (
    <AbsoluteFill
      style={{
        backgroundColor: colors.graphite,
        justifyContent: "center",
        padding: "0 140px",
      }}
    >
      <div
        style={{
          fontFamily: mono,
          fontSize: 22,
          letterSpacing: "0.28em",
          textTransform: "uppercase",
          color: accent,
          marginBottom: 28,
        }}
      >
        {kicker}
      </div>
      <div
        style={{
          fontFamily: display,
          fontWeight: 700,
          fontSize: 84,
          lineHeight: 1.05,
          color: colors.ledger,
          maxWidth: 1500,
        }}
      >
        <div>
          <TypeLine text={line1} />
        </div>
        <div style={{ color: colors.paper, marginTop: 8 }}>
          <TypeLine text={line2} delay={line2Delay} />
        </div>
      </div>
    </AbsoluteFill>
  );
};

export const CaptionBar: FC<{
  left: string;
  right: string;
  tone?: "naive" | "dotall";
}> = ({ left, right, tone = "dotall" }) => {
  const color = tone === "naive" ? colors.rust : colors.moss;
  return (
    <div
      style={{
        position: "absolute",
        left: 64,
        right: 64,
        bottom: 48,
        display: "flex",
        justifyContent: "space-between",
        alignItems: "baseline",
        gap: 32,
        fontFamily: mono,
        fontSize: 28,
        color: colors.paper,
        background: "rgba(12,13,11,0.82)",
        borderLeft: `6px solid ${color}`,
        padding: "18px 28px",
      }}
    >
      <span>{left}</span>
      <span style={{ color: colors.mute, fontSize: 22 }}>{right}</span>
    </div>
  );
};
