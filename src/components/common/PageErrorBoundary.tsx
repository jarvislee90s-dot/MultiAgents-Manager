import { Component, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
interface Props {
  children: ReactNode;
}
interface State {
  hasError: boolean;
}

// 类组件内使用翻译：错误视图抽为函数组件（useTranslation 仅限函数组件）
function PageErrorFallback({ onRetry }: { onRetry: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="flex h-full flex-col items-center justify-center gap-4 p-8">
      <p className="text-lg font-medium">{t("errorBoundary.pageCrashed")}</p>
      <div className="flex gap-2">
        <Button onClick={onRetry}>{t("common.retry")}</Button>
        <Button variant="outline" onClick={() => window.location.reload()}>
          {t("common.refreshPage")}
        </Button>
      </div>
    </div>
  );
}

export class PageErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false };
  static getDerivedStateFromError(): State {
    return { hasError: true };
  }
  render() {
    if (this.state.hasError) {
      return <PageErrorFallback onRetry={() => this.setState({ hasError: false })} />;
    }
    return this.props.children;
  }
}
