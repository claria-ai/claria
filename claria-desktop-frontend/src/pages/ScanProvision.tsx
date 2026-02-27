import StepIndicator from "../components/StepIndicator";
import type { Page } from "../App";

export default function ScanProvision({
  navigate,
}: {
  navigate: (page: Page) => void;
}) {
  return (
    <div className="max-w-2xl mx-auto p-8">
      <StepIndicator current={4} />

      <h2 className="text-2xl font-bold mb-6">Step 4: Scan & Provision</h2>

      <div className="bg-gray-50 border border-gray-200 rounded-lg p-8 text-center">
        <p className="text-gray-600 mb-2">
          Provisioner integration coming soon.
        </p>
        <p className="text-gray-500 text-sm">
          Your config has been saved. When provisioner integration is complete,
          this screen will scan your AWS account and set up all required
          resources.
        </p>
      </div>

      <div className="flex justify-between mt-8">
        <button
          onClick={() => navigate("credentials")}
          className="px-4 py-2 text-gray-600 hover:text-gray-800"
        >
          Back
        </button>
        <button
          onClick={() => navigate("dashboard")}
          className="px-6 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors"
        >
          Go to Dashboard
        </button>
      </div>
    </div>
  );
}
