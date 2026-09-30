import { cva } from "class-variance-authority";
export const inputVariant = cva(
  "flex w-full rounded-field border border-input bg-card transition-colors focus-visible:border-primary/60 focus-visible:ring-[3px] focus-visible:ring-primary/15 file:border-0 file:bg-transparent file:text-sm file:font-medium file:text-foreground placeholder:text-muted-foreground focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-50 dark:border-input",
  {
    variants: {
      density: {
        Small: "h-7 px-2 py-1 text-xs",
        Medium: "h-8 px-3 py-1.5 text-sm",
        Large: "h-11 px-4 py-3 text-base",
      },
    },
    defaultVariants: {
      density: "Medium",
    },
  },
);
