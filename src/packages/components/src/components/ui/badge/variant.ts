import { cva } from "class-variance-authority";

export const badgeVariant = cva(
  "inline-flex items-center gap-1.5 rounded-full border font-medium leading-none whitespace-nowrap transition-colors focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2",
  {
    variants: {
      variant: {
        primary: "border-transparent bg-ivy-green-tint-bg text-ivy-green-tint-fg",
        secondary: "border-transparent bg-secondary text-secondary-foreground",
        destructive: "border-transparent bg-destructive-tint-bg text-destructive-tint-fg",
        outline: "text-foreground",
        success: "border-transparent bg-success-tint-bg text-success-tint-fg",
        warning: "border-transparent bg-warning-tint-bg text-warning-tint-fg",
        info: "border-transparent bg-info-tint-bg text-info-tint-fg",
        /**
         * A badge in an arbitrary Ivy `Colors` name — the framework's `BadgeColorMapping`, which is how
         * V1 colours its Jobs Status, Type and Project columns from `Constants.JobStatusColors` and
         * friends. The fill and the text come from `--badge-tint-*`, which `Badge`'s `color` prop sets
         * per element from that name; the tokens exist for exactly this and `styles/index.css` defines
         * the utility.
         *
         * Selected by passing `color`, not by naming the variant: without a colour there is nothing to
         * tint and the badge would render on the fallback `--muted`.
         */
        tinted: "border-transparent badge-tinted",
      },
      density: {
        Small: "h-[18px] px-1.5 text-2xs",
        Medium: "h-5 px-2 text-[11px]",
        Large: "h-6 px-2.5 text-xs",
      },
    },
    defaultVariants: {
      variant: "primary",
      density: "Medium",
    },
  },
);
