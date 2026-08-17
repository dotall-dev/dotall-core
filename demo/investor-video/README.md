# Investor demo — Remotion wrapper

Storyboard for the 60–90s bake-off link. Pattern from the
[Cursor Agent Skills announcement](https://www.remotion.dev/prompts/cursor-agent-skills-announcement):
typewriter slates, a ken-burns “take,” burned-in captions, end card.

This composition is a **placeholder for live Cursor recordings**. Do not ship it
as the bake-off. Film naive + Dotall MCP on `demo/q3-pack/`, then replace
`NaiveTake` / `DotallTake` with `OffthreadVideo`.

```bash
cd demo/investor-video
npm install
npm run studio          # scrub the 21s cut
npm run render          # writes out/investor-demo.mp4
```

Drop recordings in `public/naive.mp4` and `public/dotall.mp4`, then in
`InvestorDemo.tsx`:

```tsx
import { OffthreadVideo, staticFile } from "remotion";

<OffthreadVideo src={staticFile("naive.mp4")} />
```

Keep the slates and `CaptionBar`. Do not speed up the naive take.
