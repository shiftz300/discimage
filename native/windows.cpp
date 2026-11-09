// Windows owns the optical reservation; Rust owns MMC policy and journal state.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <imapi2.h>
#include <imapi2error.h>
#include <wrl/client.h>
#include <algorithm>
#include <cstring>
#include <memory>
#include <new>
using Microsoft::WRL::ComPtr;

#define DI_TRY(call) do { HRESULT di_hresult = (call); if (FAILED(di_hresult)) return di_hresult; } while (false)

struct Apartment {
    HRESULT result = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    ~Apartment() { if (SUCCEEDED(result)) CoUninitialize(); }
};
struct BString {
    BSTR value = nullptr;
    ~BString() { SysFreeString(value); }
};
struct Array {
    SAFEARRAY* value = nullptr;
    ~Array() { if (value) SafeArrayDestroy(value); }
};
struct Session {
    Apartment apartment;
    ComPtr<IDiscRecorder2> recorder;
    ComPtr<IDiscRecorder2Ex> extended;
    BYTE* buffer = nullptr;
    ULONG maximum = 0;
    LARGE_INTEGER epoch{}, frequency{};
    bool reserved = false;
    bool mcn_disabled = false;
    ~Session() {
        if (mcn_disabled) recorder->EnableMcn();
        if (reserved) recorder->ReleaseExclusiveAccess();
        if (buffer) VirtualFree(buffer, 0, MEM_RELEASE);
        extended.Reset(); recorder.Reset(); // Before CoUninitialize.
    }
};

static HRESULT find_recorder(const wchar_t* drive, IDiscRecorder2** output) {
    ComPtr<IDiscMaster2> master;
    DI_TRY(CoCreateInstance(__uuidof(MsftDiscMaster2), nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&master)));
    LONG count = 0;
    DI_TRY(master->get_Count(&count));
    for (LONG index = 0; index < count; ++index) {
        BString id;
        DI_TRY(master->get_Item(index, &id.value));
        ComPtr<IDiscRecorder2> recorder;
        DI_TRY(CoCreateInstance(__uuidof(MsftDiscRecorder2), nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&recorder)));
        DI_TRY(recorder->InitializeDiscRecorder(id.value));
        Array volumes;
        DI_TRY(recorder->get_VolumePathNames(&volumes.value));
        LONG first = 0, last = -1;
        DI_TRY(SafeArrayGetLBound(volumes.value, 1, &first));
        DI_TRY(SafeArrayGetUBound(volumes.value, 1, &last));
        for (LONG at = first; at <= last; ++at) {
            VARIANT volume; VariantInit(&volume);
            HRESULT result = SafeArrayGetElement(volumes.value, &at, &volume);
            bool matches = SUCCEEDED(result) && volume.vt == VT_BSTR && volume.bstrVal
                && SysStringLen(volume.bstrVal) == 3 && volume.bstrVal[2] == L'\\'
                && _wcsnicmp(volume.bstrVal, drive, 2) == 0;
            VariantClear(&volume);
            DI_TRY(result);
            if (matches) { *output = recorder.Detach(); return S_OK; }
        }
    }
    return HRESULT_FROM_WIN32(ERROR_NOT_FOUND);
}

extern "C" HRESULT discimage_windows_open(const wchar_t* drive, void** output) {
    if (!output || !drive || wcslen(drive) != 2 || drive[1] != L':') return E_INVALIDARG;
    *output = nullptr;
    std::unique_ptr<Session> session(new (std::nothrow) Session());
    if (!session) return E_OUTOFMEMORY;
    DI_TRY(session->apartment.result);
    DI_TRY(find_recorder(drive, &session->recorder));
    DI_TRY(session->recorder.As(&session->extended));
    DI_TRY(session->extended->GetMaximumPageAlignedTransferSize(&session->maximum));
    ULONG mask = 0;
    DI_TRY(session->extended->GetByteAlignmentMask(&mask));
    // VirtualAlloc has allocation-granularity alignment (at least 64 KiB).
    if (mask > 65535) return HRESULT_FROM_WIN32(ERROR_NOT_SUPPORTED);
    session->maximum = std::min<ULONG>(session->maximum, 65536);
    session->buffer = static_cast<BYTE*>(VirtualAlloc(nullptr, 65536, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE));
    if (!session->buffer) return HRESULT_FROM_WIN32(GetLastError());
    if (!QueryPerformanceFrequency(&session->frequency) || session->frequency.QuadPart <= 0
        || !QueryPerformanceCounter(&session->epoch)) return E_FAIL;
    *output = session.release();
    return S_OK;
}

extern "C" HRESULT discimage_windows_reserve(void* handle) {
    if (!handle) return E_INVALIDARG;
    auto* session = static_cast<Session*>(handle);
    if (session->reserved) return S_OK;
    BString client; client.value = SysAllocString(L"discimage");
    if (!client.value) return E_OUTOFMEMORY;
    // Never force ownership away from another application.
    DI_TRY(session->recorder->AcquireExclusiveAccess(VARIANT_FALSE, client.value));
    session->reserved = true;
    DI_TRY(session->recorder->DisableMcn());
    session->mcn_disabled = true;
    return S_OK;
}

extern "C" HRESULT discimage_windows_command(void* handle, BYTE* cdb, ULONG cdb_size,
    BYTE* data, ULONG capacity, bool write, ULONG timeout, ULONG* used, BYTE* status, BYTE* sense, double* times) {
    if (!handle || !cdb || cdb_size < 6 || cdb_size > 16 || !used || !status || !sense || !times) return E_INVALIDARG;
    auto* session = static_cast<Session*>(handle);
    if (capacity > session->maximum || (capacity && !data) || timeout == 0) return E_INVALIDARG;
    *used = 0; *status = 0; std::memset(sense, 0, 32);
    std::memset(session->buffer, 0, capacity);
    if (write && !session->reserved) return E_ACCESSDENIED;
    if (write && capacity) {
        std::memcpy(session->buffer, data, capacity);
    }
    HRESULT result;
    LARGE_INTEGER start{}, end{};
    if (!QueryPerformanceCounter(&start)) return E_FAIL;
    if (!capacity) {
        result = session->extended->SendCommandNoData(cdb, cdb_size, sense, timeout);
    } else if (write) {
        result = session->extended->SendCommandSendDataToDevice(cdb, cdb_size, sense, timeout, session->buffer, capacity);
    } else {
        result = session->extended->SendCommandGetDataFromDevice(cdb, cdb_size, sense, timeout, session->buffer, capacity, used);
    }
    if (!QueryPerformanceCounter(&end) || end.QuadPart < start.QuadPart) return E_FAIL;
    times[0] = static_cast<double>(end.QuadPart - session->epoch.QuadPart) / static_cast<double>(session->frequency.QuadPart);
    times[1] = static_cast<double>(end.QuadPart - start.QuadPart) / static_cast<double>(session->frequency.QuadPart);
    if (*used > capacity) return E_UNEXPECTED;
    if (result == S_OK && capacity) {
        if (write) *used = capacity;
        else std::memcpy(data, session->buffer, *used);
    }
    if (result == S_IMAPI_COMMAND_HAS_SENSE_DATA) {
        *status = 2; *used = 0; return S_OK;
    }
    return result;
}

extern "C" void discimage_windows_close(void* handle) { delete static_cast<Session*>(handle); }
