import { ButtonHTMLAttributes } from "react";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary";
}

export function Button({
  variant = "primary",
  className = "",
  ...props
}: ButtonProps) {
  const base =
    "inline-flex items-center justify-center rounded-md px-4 py-2 text-sm font-medium transition " +
    "focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 " +
    "disabled:opacity-40 disabled:cursor-not-allowed";
  const style =
    variant === "primary"
      ? "bg-blue-600 text-white hover:bg-blue-700 active:bg-blue-800"
      : "border border-zinc-300 bg-transparent text-zinc-700 hover:bg-zinc-100 " +
        "dark:border-zinc-600 dark:text-zinc-200 dark:hover:bg-zinc-700";
  return <button className={`${base} ${style} ${className}`} {...props} />;
}