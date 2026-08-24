import { Component, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
interface Props {
  children: ReactNode;
  fallback?: ReactNode;
}
interface State {
  hasError: boolean;
}

// 类组件内使用翻译：默认错误视图抽为函数组件（useTranslation 仅限函数组件）
function DefaultErrorFallback({ onRetry }: { onRetry: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col items-center gap-2 p-4 text-center">
      <p className="text-muted-foreground text-sm">{t("errorBoundary.componentFailed")}</p>
      <Button size="sm" variant="outline" onClick={onRetry}>
        {t("common.retry")}
      </Button>
    </div>
  );
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false };
  static getDerivedStateFromError(): State {
    return { hasError: true };
  }
  render() {
    if (this.state.hasError) {
      return (
        this.props.fallback ?? (
          <DefaultErrorFallback onRetry={() => this.setState({ hasError: false })} />
        )
      );
    }
    return this.props.children;
  }
}
