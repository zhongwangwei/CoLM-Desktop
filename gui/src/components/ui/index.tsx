import { useId, useState, type ComponentProps, type ReactNode } from 'react';
import { Slot, Dialog as RadixDialog } from 'radix-ui';
import { cva } from 'class-variance-authority';
import { cn } from '../../lib/utils';
import { useApp } from '../../lib/app-state';
const buttonStyle = cva('button', {
  variants: {
    variant: {
      primary: 'button-primary',
      secondary: 'button-secondary',
      ghost: 'button-ghost',
      danger: 'button-danger',
    },
  },
  defaultVariants: { variant: 'secondary' },
});
export function Button({
  className,
  variant,
  asChild = false,
  type = 'button',
  ...props
}: ComponentProps<'button'> & {
  variant?: 'primary' | 'secondary' | 'ghost' | 'danger';
  asChild?: boolean;
}) {
  const Comp = asChild ? Slot.Root : 'button';
  return <Comp type={type} className={cn(buttonStyle({ variant }), className)} {...props} />;
}
export function Input({ className, ...props }: ComponentProps<'input'>) {
  return <input className={cn('input', className)} {...props} />;
}
export function Select({ className, ...props }: ComponentProps<'select'>) {
  return <select className={cn('input', className)} {...props} />;
}
export function Card({
  title,
  description,
  children,
  className,
}: {
  title?: ReactNode;
  description?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn('card', className)}>
      {title && <h2>{title}</h2>}
      {description && <p className="muted">{description}</p>}
      {children}
    </section>
  );
}
export function Field({
  label,
  children,
  hint,
}: {
  label: ReactNode;
  children: ReactNode;
  hint?: ReactNode;
}) {
  return (
    <label className="field">
      <span className="field-label">{label}</span>
      {children}
      {hint && <span className="field-hint">{hint}</span>}
    </label>
  );
}
export function Advanced({
  title,
  children,
  open,
  onOpenChange,
}: {
  title?: ReactNode;
  children: ReactNode;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}) {
  const { state } = useApp();
  const [own, setOwn] = useState(state.expert);
  return (
    <details
      className="advanced"
      open={open ?? own}
      onToggle={(e) => {
        const value = e.currentTarget.open;
        if (onOpenChange) onOpenChange(value);
        else setOwn(value);
      }}
    >
      <summary>{title ?? (state.language === 'en' ? 'Advanced' : '高级设置')}</summary>
      <div className="advanced-content">{children}</div>
    </details>
  );
}
export function Notice({
  children,
  tone = 'info',
}: {
  children: ReactNode;
  tone?: 'info' | 'error' | 'warning' | 'success';
}) {
  return (
    <div className={`notice notice-${tone}`} role={tone === 'error' ? 'alert' : 'status'}>
      {children}
    </div>
  );
}
export function Dialog({
  title,
  description,
  open,
  onOpenChange,
  children,
}: {
  title: ReactNode;
  description?: ReactNode;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  children: ReactNode;
}) {
  const id = useId();
  return (
    <RadixDialog.Root open={open} onOpenChange={onOpenChange}>
      <RadixDialog.Portal>
        <RadixDialog.Overlay className="dialog-overlay" />
        <RadixDialog.Content
          className="dialog-content"
          aria-describedby={description ? id : undefined}
        >
          <RadixDialog.Title>{title}</RadixDialog.Title>
          {description && <RadixDialog.Description id={id}>{description}</RadixDialog.Description>}
          {children}
          <RadixDialog.Close asChild>
            <Button variant="ghost">关闭 / Close</Button>
          </RadixDialog.Close>
        </RadixDialog.Content>
      </RadixDialog.Portal>
    </RadixDialog.Root>
  );
}
