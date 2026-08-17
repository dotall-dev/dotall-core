import type { FC } from "react";
import { AbsoluteFill, interpolate, useCurrentFrame } from "remotion";
import { CaptionBar } from "./cards";
import { colors } from "./theme";
import { display, mono } from "./type";

const tools = [
  { name: "dotall_inspect", detail: "four files · cached models" },
  { name: "dotall_search", detail: 'query "Rate" · selector named_ranges' },
  { name: "dotall_read", detail: "Inputs!B2 · quote, don't unzip" },
  { name: "dotall_edit", detail: "set_cell_value Rate 0.15" },
  { name: "dotall_apply", detail: "surgical ZIP patch · formula kept" },
];

export const DotallTake: FC = () => {
  const frame = useCurrentFrame();
  const shown = Math.min(tools.length, Math.floor(frame / 18) + 1);

  return (
    <AbsoluteFill
      style={{
        backgroundColor: colors.graphite,
        padding: "80px 90px 140px",
      }}
    >
      <div
        style={{
          fontFamily: display,
          fontSize: 42,
          color: colors.paper,
          marginBottom: 36,
        }}
      >
        Live MCP · same brief
      </div>
      {tools.slice(0, shown).map((tool, index) => {
        const opacity = interpolate(frame, [index * 18, index * 18 + 8], [0, 1], {
          extrapolateLeft: "clamp",
          extrapolateRight: "clamp",
        });
        return (
          <div
            key={tool.name}
            style={{
              opacity,
              display: "flex",
              alignItems: "baseline",
              gap: 28,
              padding: "14px 0",
              borderBottom: `1px solid ${colors.mute}33`,
              fontFamily: mono,
            }}
          >
            <span style={{ color: colors.rate, width: 64 }}>0{index + 1}</span>
            <span style={{ color: colors.ledger, fontSize: 34, width: 360 }}>
              {tool.name}
            </span>
            <span style={{ color: colors.mute, fontSize: 28 }}>{tool.detail}</span>
          </div>
        );
      })}
      <CaptionBar
        left="With Dotall — inspect → search → edit → apply"
        right="named range Rate · not ZIP XML"
      />
    </AbsoluteFill>
  );
};
