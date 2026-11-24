import { useId } from "react";

export function SolarisEmblem({ size = 26 }: { size?: number }) {
  const gradId = useId();
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 64 64"
      fill="none"
      aria-hidden="true"
      className="shrink-0"
    >
      <defs>
        <linearGradient
          id={gradId}
          x1="8"
          y1="8"
          x2="56"
          y2="56"
          gradientUnits="userSpaceOnUse"
        >
          <stop offset="0%" stopColor="#F7B733" />
          <stop offset="52%" stopColor="#F05A00" />
          <stop offset="100%" stopColor="#C31818" />
        </linearGradient>
      </defs>
      <circle cx="32" cy="32" r="26" fill={`url(#${gradId})`} />
      <rect x="4" y="29" width="56" height="6" fill="#080404" />
      <circle cx="32" cy="32" r="25.5" stroke="rgba(247,245,239,0.22)" />
    </svg>
  );
}

export function SolarisOrb({ size = 72 }: { size?: number }) {
  return (
    <div
      className="relative grid place-items-center solar-orb-breathe"
      style={{ width: size, height: size }}
      aria-hidden="true"
    >
      <div
        className="absolute inset-0 rounded-full"
        style={{
          background:
            "radial-gradient(circle, rgba(240,90,0,0.42) 0%, rgba(195,24,24,0.18) 55%, transparent 75%)",
          transform: "scale(1.55)",
        }}
      />
      <SolarisEmblem size={size} />
    </div>
  );
}

export function SolarisWordmark() {
  return (
    <div className="flex items-center gap-2.5 select-none">
      <SolarisEmblem size={24} />
      <span className="text-xs font-medium uppercase tracking-[0.3em] bg-gradient-to-r from-areia via-areia/95 to-amarelo-ideias bg-clip-text text-transparent">
        SOLARIS
      </span>
    </div>
  );
}
