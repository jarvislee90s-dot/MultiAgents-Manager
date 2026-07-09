import { Component, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
interface Props { children: ReactNode; fallback?: ReactNode; }
interface State { hasError: boolean; }
export class ErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false };
  static getDerivedStateFromError(): State { return { hasError: true }; }
  render() {
    if (this.state.hasError) {
      return this.props.fallback ?? (
        <div className="flex flex-col items-center gap-2 p-4 text-center">
          <p className="text-sm text-muted-foreground">组件加载失败</p>
          <Button size="sm" variant="outline" onClick={() => this.setState({ hasError: false })}>重试</Button>
        </div>
      );
    }
    return this.props.children;
  }
}
