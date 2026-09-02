import { ButtonHTMLAttributes } from "react";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary" | "ghost";
}

/**
 * Catalog component #1 (Solid Primary) + #2 (Ghost/Outline), adapted to the
 * Ocean Dark palette. All micro-interactions required by the design rules:
 * hover shift, active scale 0.98, focus ring, 150ms transitions.
 */
export function Button({
  variant = "primary",
  className = "",
  ...props
}: ButtonProps) {
  const base =
    "inline-flex items-center justify-center gap-2 rounded-lg px-5 py-2.5 text-sm font-semibold " +
    "transition-all duration-150 focus:outline-none focus-visible:ring-2 " +
    "focus-visible:ring-sky-400 focus-visible:ring-offset-2 focus-visible:ring-offset-transparent " +
    "disabled:opacity-40 disabled:cursor-not-allowed disabled:active:scale-100";
  const style =
    variant === "primary"
      ? "bg-sky-500 text-white shadow-sm shadow-sky-500/25 hover:bg-sky-400 " +
        "hover:shadow-md hover:shadow-sky-500/30 active:scale-[0.98] active:bg-sky-600"
      : variant === "secondary"
        ? "border border-slate-300 bg-transparent font-medium text-slate-700 hover:bg-slate-100 " +
          "active:scale-[0.98] dark:border-slate-600 dark:text-slate-200 dark:hover:bg-slate-700/50"
        : "font-medium text-slate-500 hover:bg-slate-200/60 hover:text-slate-700 " +
          "active:scale-[0.98] dark:text-slate-400 dark:hover:bg-slate-700/40 dark:hover:text-slate-200";
  return <button className={`${base} ${style} ${className}`} {...props} />;
}
