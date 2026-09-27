import type { ReactNode } from "react";

type Props = {
  title: string;
  context?: ReactNode;
  actions?: ReactNode;
  className?: string;
};

export function PageHeader({ title, context, actions, className = "" }: Props) {
  return (
    <header className={"page-heading " + className}>
      <div className="page-heading-copy">
        <h1>{title}</h1>
        {context && <div className="page-heading-context">{context}</div>}
      </div>
      {actions && <div className="page-heading-actions">{actions}</div>}
    </header>
  );
}
