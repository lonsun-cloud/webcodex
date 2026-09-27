import type { ButtonHTMLAttributes, ReactNode } from "react";

type Props = Omit<ButtonHTMLAttributes<HTMLButtonElement>, "children"> & {
  label: string;
  children: ReactNode;
};

export function IconButton({ label, children, className = "", title = label, type = "button", ...props }: Props) {
  return <button {...props} type={type} className={"icon-button " + className} aria-label={label} title={title}>{children}</button>;
}
