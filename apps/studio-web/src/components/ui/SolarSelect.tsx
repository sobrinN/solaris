import type { SelectHTMLAttributes } from "react";
import { ChevronDown } from "lucide-react";

interface SolarSelectProps extends SelectHTMLAttributes<HTMLSelectElement> {
  variant?: "pill" | "field" | "ghost";
  wrapperClassName?: string;
}

const VARIANT_CLASSES: Record<NonNullable<SolarSelectProps["variant"]>, string> = {
  pill: "h-9 rounded-full border border-areia/12 bg-grafite/75 pl-3 pr-7 text-xs text-areia hover:border-areia/25 focus:border-laranja-criacao",
  field:
    "h-9 w-full rounded-xl border border-areia/14 bg-void/75 pl-3 pr-8 text-xs text-areia hover:border-areia/25 focus:border-laranja-criacao",
  ghost:
    "h-7 rounded-full border border-transparent bg-transparent pl-2 pr-6 text-[11px] font-medium text-areia hover:bg-areia/[0.06] focus:border-laranja-criacao/60",
};

export function SolarSelect({
  variant = "pill",
  className = "",
  wrapperClassName = "",
  children,
  ...rest
}: SolarSelectProps) {
  return (
    <div className={`relative inline-flex items-center ${wrapperClassName}`}>
      <select
        {...rest}
        className={`appearance-none truncate outline-none transition-colors motion-reduce:transition-none disabled:opacity-45 ${VARIANT_CLASSES[variant]} ${className}`}
      >
        {children}
      </select>
      <ChevronDown
        aria-hidden="true"
        className={`pointer-events-none absolute text-areia/55 ${
          variant === "ghost" ? "right-1.5 h-3 w-3" : "right-2.5 h-3.5 w-3.5"
        }`}
      />
    </div>
  );
}
