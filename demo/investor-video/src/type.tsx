import type { FC } from "react";
import { loadFont as loadMono } from "@remotion/google-fonts/IBMPlexMono";
import { loadFont as loadDisplay } from "@remotion/google-fonts/Syne";
import { useCurrentFrame } from "remotion";

export const { fontFamily: mono } = loadMono("normal", {
  weights: ["400", "500"],
  subsets: ["latin"],
  ignoreTooManyRequestsWarning: true,
});
export const { fontFamily: display } = loadDisplay("normal", {
  weights: ["700"],
  subsets: ["latin"],
  ignoreTooManyRequestsWarning: true,
});

export const TypeLine: FC<{
  text: string;
  delay?: number;
  cps?: number;
}> = ({ text, delay = 0, cps = 1 }) => {
  const frame = useCurrentFrame();
  const typed = Math.max(0, frame - delay);
  const count = Math.min(text.length, Math.floor(typed / cps));
  const shown = text.slice(0, count);
  const typing = count < text.length;
  const caret = typing && typed % 16 < 10 ? "|" : "";
  return (
    <span>
      {shown}
      {caret}
    </span>
  );
};
