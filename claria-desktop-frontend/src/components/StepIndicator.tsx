const STEPS = [
  "AWS Account",
  "Root MFA",
  "Access Key",
  "Credentials",
  "Provision",
];

export default function StepIndicator({ current }: { current: number }) {
  return (
    <div className="flex items-center justify-center gap-2 mb-8">
      {STEPS.map((label, i) => {
        const step = i + 1;
        const active = step === current;
        const done = step < current;
        return (
          <div key={label} className="flex items-center gap-2">
            {i > 0 && (
              <div
                className={`w-8 h-0.5 ${
                  done ? "bg-blue-500" : "bg-gray-300"
                }`}
              />
            )}
            <div className="flex flex-col items-center">
              <div
                className={`w-8 h-8 rounded-full flex items-center justify-center text-sm font-medium ${
                  active
                    ? "bg-blue-500 text-white"
                    : done
                      ? "bg-blue-100 text-blue-700"
                      : "bg-gray-200 text-gray-500"
                }`}
              >
                {step}
              </div>
              <span className="text-xs text-gray-500 mt-1">{label}</span>
            </div>
          </div>
        );
      })}
    </div>
  );
}
