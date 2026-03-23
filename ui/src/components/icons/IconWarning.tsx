import type { IconProps } from "../../types";

export function IconWarning({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" className={className}>
      <path
        d="M8 1.5L1.5 13h13L8 1.5Z"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinejoin="round"
      />
      <path d="M8 6v3" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
      <circle cx="8" cy="11" r="0.5" fill="currentColor" />
    </svg>
  );
}
