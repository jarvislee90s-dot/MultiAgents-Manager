import { useCallback, useState } from "react";
import Board from "./Board";
import PairPage from "./PairPage";

// 配对状态机（轮询全部由 Board 自持，App 只持状态标记）：
// - null  探测中：首帧即出配对页（沿用 Task 5 不闪白口径），Board 在底下挂载完成首次探测；
//         已配对设备（cookie 有效）由 Board 首拍拉到数据后翻转为 true，配对页随即卸载
// - true  已配对：仅 Board（配对页卸载）
// - false 设备失效：Board 收 403 回调后置 false 并卸载（停轮询），回配对页；
//         重新配对成功 → true → Board 重新挂载恢复轮询
// 注意：网络异常不走 403 通道，Board 内部保数据重试，不会误置 false
export default function App() {
  const [paired, setPaired] = useState<boolean | null>(null);
  const onPaired = useCallback(() => setPaired(true), []);
  const onUnpaired = useCallback(() => setPaired(false), []);

  return (
    <>
      {/* Board 常驻持轮询（探测也来自轮询首拍）；未配对态仅隐藏，避免与配对页堆叠 */}
      {paired !== false && (
        <div className={paired === true ? "contents" : "hidden"}>
          <Board onUnpaired={onUnpaired} />
        </div>
      )}
      {paired !== true && <PairPage onPaired={onPaired} />}
    </>
  );
}
