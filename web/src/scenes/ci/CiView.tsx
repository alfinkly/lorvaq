import { useEffect, useRef } from "react";
import { follow } from "../../api";
import { CiRenderer } from "./renderer";

interface Props {
  onStatus: (status: "live" | "reconnecting") => void;
  onStale: (stale: Record<string, number>) => void;
}

export function CiView({ onStatus, onStale }: Props) {
  const ref = useRef<SVGSVGElement>(null);
  useEffect(() => {
    const svg = ref.current;
    if (!svg) return;
    const renderer = new CiRenderer(svg);
    let lastStale = "";
    const sub = follow(
      "ci",
      (world, effects) => {
        renderer.update(world, effects);
        const stale = JSON.stringify([...world.stale]);
        if (stale !== lastStale) {
          lastStale = stale;
          onStale(Object.fromEntries(world.stale));
        }
      },
      onStatus,
    );
    return () => {
      sub.close();
      renderer.destroy();
    };
  }, [onStatus, onStale]);
  return <svg ref={ref} className="scene" role="img" aria-label="CI scene" />;
}
