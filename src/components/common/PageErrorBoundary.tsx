import { Component, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
interface Props { children: ReactNode; }
interface State { hasError: boolean; }
export class PageErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false };
  static getDerivedStateFromError(): State { return { hasError: true }; }
  render() {
    if (this.state.hasError) {
      return (
        <div className="flex h-full flex-col items-center justify-center gap-4 p-8">
          <p className="text-lg font-medium">出错了</p>
          <div className="flex gap-2">
            <Button onClick={() => this.setState({ hasError: false })}>重试</Button>
            <Button variant="outline" onClick={() => window.location.reload()}>刷新页面</Button>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}
