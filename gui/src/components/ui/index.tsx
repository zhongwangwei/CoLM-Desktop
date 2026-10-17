import {
  Children,
  Fragment,
  cloneElement,
  isValidElement,
  useId,
  useState,
  useRef,
  type ComponentProps,
  type ReactNode,
  type ReactElement,
} from 'react';
import { Slot, Dialog as RadixDialog, ContextMenu as RadixContextMenu } from 'radix-ui';
import { cva } from 'class-variance-authority';
import { cn } from '../../lib/utils';
import { useApp } from '../../lib/app-state';
import { validDate } from '../../lib/dates';
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
export function DateInput({
  value,
  onChange,
  disabled,
  readOnly,
  calendarLabel,
  ...props
}: Omit<ComponentProps<'input'>, 'type' | 'value'> & { value: string; calendarLabel: string }) {
  const picker = useRef<HTMLInputElement>(null);
  const [nativeVisible, setNativeVisible] = useState(false);
  function openCalendar() {
    if (disabled || readOnly || !picker.current) return;
    // Older WebKit can silently ignore showPicker, so keep its date control reachable.
    setNativeVisible(true);
    picker.current.focus({ preventScroll: true });
    try {
      if (typeof picker.current.showPicker === 'function') {
        picker.current.showPicker();
      }
    } catch {
      // Older webviews can still expose their ordinary native date control.
    }
  }
  return (
    <div
      className="browse-input"
      style={{ position: 'relative', flexWrap: nativeVisible ? 'wrap' : undefined }}
    >
      <Input
        {...props}
        type="text"
        value={value}
        disabled={disabled}
        readOnly={readOnly}
        placeholder={props.placeholder ?? 'YYYY-MM-DD'}
        maxLength={props.maxLength ?? 10}
        onChange={(event) => {
          onChange?.(event);
          setNativeVisible(false);
        }}
      />
      <Button
        disabled={disabled || readOnly}
        aria-label={calendarLabel}
        title={calendarLabel}
        onClick={openCalendar}
      >
        <svg
          width="18"
          height="18"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          aria-hidden="true"
        >
          <rect x="3" y="5" width="18" height="16" rx="2" />
          <path d="M16 3v4M8 3v4M3 11h18" />
        </svg>
      </Button>
      <input
        ref={picker}
        className="input"
        type="date"
        aria-label={calendarLabel}
        aria-describedby={props['aria-describedby']}
        tabIndex={nativeVisible ? 0 : -1}
        min={props.min ?? '0001-01-01'}
        max={props.max ?? '9999-12-31'}
        value={validDate(value) ? value : ''}
        onChange={(event) => {
          onChange?.(event);
          setNativeVisible(false);
        }}
        disabled={disabled}
        readOnly={readOnly}
        style={
          nativeVisible
            ? { flexBasis: '100%' }
            : {
                position: 'absolute',
                right: 0,
                bottom: 0,
                width: 1,
                height: 1,
                opacity: 0,
                pointerEvents: 'none',
              }
        }
      />
    </div>
  );
}
export function Select({ className, ...props }: ComponentProps<'select'>) {
  return <select className={cn('input', className)} {...props} />;
}
export function ContextMenu({
  children,
  items,
  onOpenChange,
}: {
  children: ReactElement;
  items: { label: string; onSelect: () => void; disabled?: boolean; danger?: boolean }[];
  onOpenChange?: (open: boolean) => void;
}) {
  return (
    <RadixContextMenu.Root onOpenChange={onOpenChange}>
      <RadixContextMenu.Trigger asChild>{children}</RadixContextMenu.Trigger>
      <RadixContextMenu.Portal>
        <RadixContextMenu.Content className="context-menu" collisionPadding={8}>
          {items.map((item) => (
            <RadixContextMenu.Item
              key={item.label}
              className={cn('context-menu-item', item.danger && 'context-menu-danger')}
              disabled={item.disabled}
              onSelect={item.onSelect}
            >
              {item.label}
            </RadixContextMenu.Item>
          ))}
        </RadixContextMenu.Content>
      </RadixContextMenu.Portal>
    </RadixContextMenu.Root>
  );
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
  const generatedId = useId();
  let controlId: string | undefined;
  const associate = (nodes: ReactNode): ReactNode =>
    Children.map(nodes, (child) => {
      if (controlId || !isValidElement<{ id?: string; type?: string; children?: ReactNode }>(child))
        return child;
      const input = child.type === Input || child.type === DateInput || child.type === 'input';
      if (
        (input &&
          !['hidden', 'button', 'submit', 'reset', 'image'].includes(child.props.type ?? 'text')) ||
        child.type === Select ||
        child.type === 'select' ||
        child.type === 'textarea'
      ) {
        controlId = child.props.id || generatedId;
        return cloneElement(child, { id: controlId });
      }
      // Traverse layout wrappers, leaving independently labelled controls and components alone.
      if (
        child.type === Fragment ||
        (typeof child.type === 'string' && child.type !== 'label' && child.type !== 'button')
      )
        return cloneElement(child, { children: associate(child.props.children) });
      return child;
    });
  const content = associate(children);
  return (
    <div className="field">
      {controlId ? (
        <label className="field-label" htmlFor={controlId}>
          {label}
        </label>
      ) : (
        <span className="field-label">{label}</span>
      )}
      {content}
      {hint && <span className="field-hint">{hint}</span>}
    </div>
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
  showClose = true,
  children,
}: {
  title: ReactNode;
  description?: ReactNode;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  showClose?: boolean;
  children: ReactNode;
}) {
  const id = useId();
  const { state } = useApp();
  const closeLabel = state.language === 'en' ? 'Close' : '关闭';
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
          {showClose && (
            <RadixDialog.Close asChild>
              <Button variant="ghost">{closeLabel}</Button>
            </RadixDialog.Close>
          )}
        </RadixDialog.Content>
      </RadixDialog.Portal>
    </RadixDialog.Root>
  );
}
