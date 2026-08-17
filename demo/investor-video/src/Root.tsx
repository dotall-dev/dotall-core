import type { FC } from "react";
import { Composition } from "remotion";
import { InvestorDemo } from "./InvestorDemo";
import { fps, height, width } from "./theme";

export const RemotionRoot: FC = () => {
  return (
    <Composition
      id="InvestorDemo"
      component={InvestorDemo}
      durationInFrames={630}
      fps={fps}
      width={width}
      height={height}
    />
  );
};
