import type { FC } from "react";
import { AbsoluteFill, Easing, interpolate, useCurrentFrame } from "remotion";
import { CaptionBar } from "./cards";
import { colors } from "./theme";
import { mono } from "./type";

const grepLines = [
  ["q3-financials.xlsx", "FY2024-03!C2", "legacy 10% promo"],
  ["q3-financials.xlsx", "FY2024-07!C2", "legacy 10% promo"],
  ["q3-deck.pptx", "slide 9", "Appendix FY mix 10%"],
  ["q3-deck.pptx", "slide 14", "Appendix FY mix 10%"],
  ["q3-memo.docx", "p.18", "Prior year channel mix stayed at 10%."],
  ["q3-memo.docx", "p.22", "Prior year channel mix stayed at 10%."],
  ["q3-intake.pdf", "field Rate", "10%  ← live, buried"],
];

export const NaiveTake: FC = () => {
  const frame = useCurrentFrame();
  const scale = interpolate(frame, [0, 120], [1, 1.28], {
    easing: Easing.inOut(Easing.cubic),
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  const visible = Math.min(grepLines.length, Math.floor(frame / 8) + 1);

  return (
    <AbsoluteFill style={{ backgroundColor: colors.ink }}>
      <AbsoluteFill
        style={{
          transform: `scale(${scale})`,
          transformOrigin: "12% 8%",
        }}
      >
        <div
          style={{
            margin: 72,
            height: 780,
            background: "#0a0b09",
            border: `1px solid ${colors.mute}`,
            fontFamily: mono,
            color: colors.ledger,
            padding: 36,
            fontSize: 26,
            lineHeight: 1.45,
            overflow: "hidden",
          }}
        >
          <div style={{ color: colors.mute, marginBottom: 18 }}>
            $ unzip -p q3-financials.xlsx xl/sharedStrings.xml | grep -n 10%
          </div>
          {grepLines.slice(0, visible).map(([file, sel, snip]) => (
            <div key={`${file}-${sel}`} style={{ display: "flex", gap: 24 }}>
              <span style={{ color: colors.rust, width: 280 }}>{file}</span>
              <span style={{ color: colors.rate, width: 220 }}>{sel}</span>
              <span>{snip}</span>
            </div>
          ))}
          {visible >= grepLines.length ? (
            <div style={{ marginTop: 28, color: colors.rust }}>
              47 matches · which 10% is live?
            </div>
          ) : null}
        </div>
      </AbsoluteFill>
      <CaptionBar
        tone="naive"
        left="Without Dotall — unzip + grep"
        right="decoy 10% in history / appendix / prior-year copy"
      />
    </AbsoluteFill>
  );
};
