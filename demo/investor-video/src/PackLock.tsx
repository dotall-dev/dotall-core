import type { FC } from "react";
import { AbsoluteFill, interpolate, spring, useCurrentFrame, useVideoConfig } from "remotion";
import { CaptionBar } from "./cards";
import { colors } from "./theme";
import { display, mono } from "./type";

const files = [
  { ext: "xlsx", name: "q3-financials", live: "Rate → Inputs!B2" },
  { ext: "pptx", name: "q3-deck", live: "slide 1 · KPI" },
  { ext: "docx", name: "q3-memo", live: "Q3 status line" },
  { ext: "pdf", name: "q3-intake", live: "AcroForm Rate" },
];

export const PackLock: FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const flipped = spring({
    frame: frame - 12,
    fps,
    config: { damping: 14, mass: 0.7 },
  });
  const fifteen = interpolate(flipped, [0, 1], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });

  return (
    <AbsoluteFill
      style={{
        backgroundColor: colors.paper,
        padding: "90px 90px 140px",
      }}
    >
      <div
        style={{
          fontFamily: display,
          fontSize: 40,
          color: colors.ink,
          marginBottom: 40,
        }}
      >
        Four human files. One rate.
      </div>
      <div style={{ display: "flex", gap: 28 }}>
        {files.map((file, index) => (
          <div
            key={file.ext}
            style={{
              flex: 1,
              background: colors.graphite,
              color: colors.ledger,
              padding: 28,
              minHeight: 420,
              transform: `translateY(${interpolate(
                spring({
                  frame: frame - index * 6,
                  fps,
                  config: { damping: 16 },
                }),
                [0, 1],
                [24, 0],
              )}px)`,
            }}
          >
            <div
              style={{
                fontFamily: mono,
                fontSize: 18,
                letterSpacing: "0.18em",
                color: colors.rate,
                marginBottom: 18,
              }}
            >
              .{file.ext}
            </div>
            <div
              style={{
                fontFamily: display,
                fontSize: 32,
                marginBottom: 48,
              }}
            >
              {file.name}
            </div>
            <div style={{ fontFamily: mono, fontSize: 22, color: colors.mute }}>
              {file.live}
            </div>
            <div
              style={{
                marginTop: 36,
                fontFamily: mono,
                fontSize: 56,
                color: fifteen > 0.5 ? colors.rate : colors.rust,
              }}
            >
              {fifteen > 0.5 ? "15%" : "10%"}
            </div>
          </div>
        ))}
      </div>
      <CaptionBar
        left="xlsx · pptx · docx · pdf agree"
        right="formulas, metrics table, Word styles preserved"
      />
    </AbsoluteFill>
  );
};
