import type { ComponentPropsWithoutRef } from "react";

/**
 * Forge's mark: an anvil struck with a spark, in ember orange. Inline rather than a file, so the
 * shell never renders a blank box before an asset resolves. The same artwork (on a dark tile) is
 * `src-tauri/icons/app-icon.svg`, which the app icons are generated from.
 */
export function ForgeLogo(props: ComponentPropsWithoutRef<"svg">) {
  return (
    <svg
      viewBox="64 64 384 384"
      xmlns="http://www.w3.org/2000/svg"
      role="img"
      aria-label="Forge"
      {...props}
    >
      <defs>
        <linearGradient id="forge-ember" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#ffb347" />
          <stop offset="1" stopColor="#ff6a1a" />
        </linearGradient>
      </defs>
      <path
        fill="url(#forge-ember)"
        d="M88 212 Q150 212 178 192 L424 192 L424 246 L358 246 Q332 258 328 296 L328 326 L376 326 L394 368 L150 368 L168 326 L216 326 L216 296 Q212 260 178 250 Q122 244 88 212 Z"
      />
      <path
        fill="#ffd27a"
        d="M300 92 L312 136 L356 148 L312 160 L300 204 L288 160 L244 148 L288 136 Z"
      />
    </svg>
  );
}
