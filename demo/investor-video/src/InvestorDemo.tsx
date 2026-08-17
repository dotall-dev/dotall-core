import type { FC } from "react";
import { AbsoluteFill, Sequence } from "remotion";
import { TitleCard } from "./cards";
import { DotallTake } from "./DotallTake";
import { NaiveTake } from "./NaiveTake";
import { PackLock } from "./PackLock";
import { colors } from "./theme";

/**
 * Storyboard wrapper for the investor bake-off.
 * Swap NaiveTake / DotallTake for <OffthreadVideo src={staticFile("naive.mp4")} />
 * once the live Cursor sessions are on disk — keep the slates and caption bars.
 */
export const InvestorDemo: FC = () => {
  return (
    <AbsoluteFill style={{ backgroundColor: colors.graphite }}>
      <Sequence durationInFrames={90}>
        <TitleCard
          kicker="Northstar Analytics · Q3 board pack"
          line1="Same brief. Two agents."
          line2="One unzips the workbook."
        />
      </Sequence>
      <Sequence from={94} durationInFrames={120}>
        <NaiveTake />
      </Sequence>
      <Sequence from={210} durationInFrames={90}>
        <TitleCard
          kicker="Dotall MCP"
          line1="The other hits Rate."
          line2="Then patches four files."
          accent={colors.moss}
        />
      </Sequence>
      <Sequence from={300} durationInFrames={150}>
        <DotallTake />
      </Sequence>
      <Sequence from={450} durationInFrames={90}>
        <PackLock />
      </Sequence>
      <Sequence from={540} durationInFrames={90}>
        <TitleCard
          kicker=".all/  ·  cache · versions · revert"
          line1="Agents already try this."
          line2="Dotall doesn't break the file."
        />
      </Sequence>
    </AbsoluteFill>
  );
};
