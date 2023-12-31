use std::{intrinsics::copy_nonoverlapping, mem::size_of, rc::Rc};

#[macro_use]
extern crate static_assertions;

use cgmath::{prelude::*, Vector4};
use cgmath::{Matrix4, Vector3};
use log::trace;
use memoffset::offset_of;

use rusty_d3d12::*;

mod fps_counter;
use fps_counter::FPSCounter;

#[no_mangle]
pub static D3D12SDKVersion: u32 = 606;

#[no_mangle]
pub static D3D12SDKPath: &[u8; 9] = b".\\D3D12\\\0";

use winit::event_loop::ControlFlow;
//use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};
use winit::{
    //dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::EventLoop,
    platform::windows::WindowExtWindows,
    window::WindowBuilder,
};

const WINDOW_WIDTH: u32 = 640;
const WINDOW_HEIGHT: u32 = 480;
const FRAMES_IN_FLIGHT: u32 = 3;

const USE_DEBUG: bool = false;
const USE_WARP_ADAPTER: bool = false;

pub const PATH_TO_OBJ_MODEL: &str = "assets/stanford_dragon.obj";
// pub const PATH_TO_OBJ_MODEL: &'static str = "C://Dev//plane.obj";
// pub const PATH_TO_OBJ_MODEL: &'static str = "C://Dev//ball.obj";

struct ScopedDebugMessagePrinter {
    info_queue: Rc<InfoQueue>,
}

impl ScopedDebugMessagePrinter {
    fn new(info_queue: Rc<InfoQueue>) -> Self {
        ScopedDebugMessagePrinter { info_queue }
    }
}

impl Drop for ScopedDebugMessagePrinter {
    fn drop(&mut self) {
        self.info_queue
            .print_messages()
            .expect("Cannot print info queue messages");
    }
}

//macro_rules! make_debug_printer {
//    ($info_queue:expr) => {
//        ScopedDebugMessagePrinter::new(Rc::clone(&$info_queue));
//    };
//}

#[repr(C)]
struct Vertex {
    position: Vector3<f32>,
}

impl Vertex {
    fn make_desc() -> Vec<InputElementDesc<'static>> {
        vec![InputElementDesc::default()
            .with_semantic_name("POSITION")
            .unwrap()
            .with_format(Format::R32G32B32Float)
            .with_input_slot(0)
            .with_aligned_byte_offset(ByteCount(offset_of!(Self, position) as u64))]
    }
}

#[derive(Debug)]
#[repr(C)]
struct Meshlet {
    vertex_count: u32,
    triangles_offset: u32,
    triangle_count: u32,
    vertices_offset: u32,
}

impl Meshlet {
    const MAX_VERTEX_COUNT: usize = 64;
    const MAX_TRIANGLE_COUNT: usize = 126;
}

type Mat4 = Matrix4<f32>;
type Vec3 = Vector3<f32>;
//type Vec4 = Vector4<f32>;

#[derive(Clone, Copy)]
struct MeshletConstantBuffer {
    mvp: Mat4,
    padding: [f32; 48],
}
const_assert!(size_of::<MeshletConstantBuffer>() == 256);

struct HelloMeshShadersSample {
    device: Device,
    //debug_device: DebugDevice,
    //info_queue: Rc<InfoQueue>,
    command_queue: CommandQueue,
    fence: Fence,
    fence_event: Win32Event,
    fence_values: [u64; FRAMES_IN_FLIGHT as usize],
    swapchain: Swapchain,
    frame_index: usize,
    frame_count: u32,
    viewport_desc: Viewport,
    scissor_desc: Rect,
    render_targets: Vec<Resource>,
    rtv_heap: DescriptorHeap,
    rtv_descriptor_handle_size: ByteCount,
    dsv_heap: DescriptorHeap,
    command_allocators: Vec<CommandAllocator>,
    command_list: CommandList,
    root_signature: RootSignature,
    pso: PipelineState,

    depth_stencil: Option<Resource>,

    meshlet_count: u32,
    vertex_buffer: Option<Resource>,
    meshlet_buffer: Option<Resource>,
    triangle_indices_buffer: Option<Resource>,
    vertex_indices_buffer: Option<Resource>,

    constant_buffer_ptr: *mut u8,
    constant_buffer: Option<Resource>,
}

impl HelloMeshShadersSample {
    fn new(hwnd: *mut std::ffi::c_void) -> Self {
        trace!("Creating app instance");

        let mut factory_flags = CreateFactoryFlags::None;
        if USE_DEBUG {
            let debug_controller = Debug::new().expect("Cannot create debug controller");
            debug_controller.enable_debug_layer();
            factory_flags = CreateFactoryFlags::Debug;
        }

        let factory = Factory::new(factory_flags).expect("Cannot create factory");

        let device = create_device(&factory);

        //let debug_device = DebugDevice::new(&device).expect("Cannot create debug device");

        /*
        let info_queue = Rc::new(
            InfoQueue::new(
                &device,
                if break_on_warn {
                    Some(&[
                        MessageSeverity::Corruption,
                        MessageSeverity::Error,
                        MessageSeverity::Warning,
                    ])
                } else {
                    None
                },
            )
            .expect("Cannot create debug info queue"),
        );

        let _debug_printer = ScopedDebugMessagePrinter::new(Rc::clone(&info_queue));
        */

        let command_queue = device
            .create_command_queue(&CommandQueueDesc::default())
            .expect("Cannot create command queue");

        let fence = device
            .create_fence(0, FenceFlags::None)
            .expect("Cannot create fence");

        let fence_event = Win32Event::default();
        let fence_values = [0; FRAMES_IN_FLIGHT as usize];
        let frame_index = 0;

        let swapchain = create_swapchain(factory, &command_queue, hwnd);

        let viewport_desc = Viewport::default()
            .with_width(WINDOW_WIDTH as f32)
            .with_height(WINDOW_HEIGHT as f32);

        let scissor_desc = Rect::default()
            .with_right(WINDOW_WIDTH as i32)
            .with_bottom(WINDOW_HEIGHT as i32);

        let rtv_descriptor_handle_size =
            device.get_descriptor_handle_increment_size(DescriptorHeapType::Rtv);

        let (render_targets, rtv_heap, dsv_heap) =
            setup_heaps(&device, &swapchain, rtv_descriptor_handle_size);

        trace!("Created heaps and render targets");

        let mut command_allocators = vec![];
        for _ in 0..FRAMES_IN_FLIGHT {
            command_allocators.push(
                device
                    .create_command_allocator(CommandListType::Direct)
                    .expect("Cannot create command allocator"),
            );
        }

        let (mesh_shader, pixel_shader) = create_shaders();

        trace!("Compiled shaders");

        let root_signature = setup_root_signature(&device, &mesh_shader);

        trace!("Created root signature");

        let pso = create_pipeline_state(&root_signature, mesh_shader, pixel_shader, &device);

        trace!("Created PSO");

        let command_list = device
            .create_command_list(
                CommandListType::Direct,
                &command_allocators[0],
                Some(&pso),
                // None,
            )
            .expect("Cannot create command list");
        command_list.close().expect("Cannot close command list");

        let mut hello_mesh_shaders_sample = Self {
            device,
            //debug_device,
            //info_queue,
            command_queue,
            fence,
            fence_event,
            fence_values,
            swapchain,
            frame_index,
            frame_count: 0,
            viewport_desc,
            scissor_desc,
            render_targets,
            rtv_heap,
            rtv_descriptor_handle_size,
            dsv_heap,
            command_allocators,
            root_signature,
            pso,
            command_list,
            depth_stencil: None,

            meshlet_count: 0,
            vertex_buffer: None,
            meshlet_buffer: None,
            triangle_indices_buffer: None,
            vertex_indices_buffer: None,

            constant_buffer_ptr: std::ptr::null_mut(),
            constant_buffer: None,
        };
        trace!("Created app instance");

        hello_mesh_shaders_sample.setup_dsv();
        let (vertices, meshlets, triangle_indices, vertex_indices) = load_mesh();
        trace!("Created DSV");

        hello_mesh_shaders_sample.setup_geometry_buffers(
            vertices,
            meshlets,
            triangle_indices,
            vertex_indices,
        );
        trace!("Finished setting up geometry buffers");

        hello_mesh_shaders_sample.setup_constant_buffer();
        trace!("Finished setting up constant buffer");

        hello_mesh_shaders_sample.flush_gpu();

        hello_mesh_shaders_sample
    }

    fn setup_geometry_buffers(
        &mut self,
        vertices: Vec<Vertex>,
        meshlets: Vec<Meshlet>,
        triangle_indices: Vec<u32>,
        vertex_indices: Vec<u32>,
    ) {
        self.vertex_buffer = Some(self.create_default_buffer(&vertices, "VertexBuffer"));

        self.meshlet_count = meshlets.len() as u32;
        self.meshlet_buffer = Some(self.create_default_buffer(&meshlets, "MeshletBuffer"));
        self.triangle_indices_buffer =
            Some(self.create_default_buffer(&triangle_indices, "TriangleIndexBuffer"));
        self.vertex_indices_buffer =
            Some(self.create_default_buffer(&vertex_indices, "VertexIndexBuffer"));
    }

    fn create_default_buffer<T>(&mut self, init_data: &Vec<T>, debug_name: &str) -> Resource {
        //let _debug_printer = ScopedDebugMessagePrinter::new(Rc::clone(&self.info_queue));

        self.command_list
            .reset(&self.command_allocators[0], None)
            .expect("Cannot reset command lsit");

        let size = ByteCount::from(init_data.len() * std::mem::size_of::<T>());
        let staging_buffer = self
            .device
            .create_committed_resource(
                &HeapProperties::default().with_heap_type(HeapType::Upload),
                HeapFlags::None,
                &ResourceDesc::default()
                    .with_dimension(ResourceDimension::Buffer)
                    .with_width(size.0)
                    .with_layout(TextureLayout::RowMajor),
                ResourceStates::GenericRead,
                None,
            )
            .expect("Cannot create staging buffer");

        staging_buffer
            .set_name(&format!("Staging{}", debug_name))
            .expect("Cannot set name on staging buffer");

        let data = staging_buffer
            .map(0, None)
            .expect("Cannot map staging buffer");

        unsafe {
            std::ptr::copy_nonoverlapping(init_data.as_ptr() as *const u8, data, size.0 as usize);
        }
        staging_buffer.unmap(0, None);

        let default_buffer = self
            .device
            .create_committed_resource(
                &HeapProperties::default().with_heap_type(HeapType::Default),
                HeapFlags::None,
                &ResourceDesc::default()
                    .with_dimension(ResourceDimension::Buffer)
                    .with_width(size.0)
                    .with_layout(TextureLayout::RowMajor),
                ResourceStates::CopyDest,
                None,
            )
            .expect("Cannot create default buffer");

        default_buffer
            .set_name(&format!("Default{}", debug_name))
            .expect("Cannot set name on default buffer");

        // self.command_list
        //     .resource_barrier(&[ResourceBarrier::new_transition(
        //         &ResourceTransitionBarrier::default()
        //             .set_resource(&default_buffer)
        //             .set_state_before(ResourceStates::Common)
        //             .set_state_after(ResourceStates::CopyDest),
        //     )]);

        self.command_list.copy_buffer_region(
            &default_buffer,
            ByteCount(0),
            &staging_buffer,
            ByteCount(0),
            size,
        );

        self.command_list
            .resource_barrier(&[ResourceBarrier::new_transition(
                &ResourceTransitionBarrier::default()
                    .with_resource(&default_buffer)
                    .with_state_before(ResourceStates::CopyDest)
                    .with_state_after(ResourceStates::GenericRead),
            )]);

        self.command_list
            .close()
            .expect("Cannot close command list");

        self.command_queue
            .execute_command_lists(std::slice::from_ref(&self.command_list));
        self.flush_gpu();

        default_buffer
    }

    fn populate_command_list(&mut self, frame_index: usize) {
        self.command_allocators[frame_index]
            .reset()
            .expect("Cannot reset command allocator");

        self.command_list
            .reset(&self.command_allocators[frame_index], Some(&self.pso))
            .expect("Cannot reset command list");

        self.command_list
            .set_graphics_root_signature(&self.root_signature);

        self.command_list.set_graphics_root_constant_buffer_view(
            0,
            self.constant_buffer
                .as_ref()
                .expect("No constant buffer created")
                .get_gpu_virtual_address(),
        );
        self.command_list.set_graphics_root_shader_resource_view(
            1,
            self.vertex_buffer
                .as_ref()
                .expect("No vertex buffer created")
                .get_gpu_virtual_address(),
        );
        self.command_list.set_graphics_root_shader_resource_view(
            2,
            self.meshlet_buffer
                .as_ref()
                .expect("No meshlet buffer created")
                .get_gpu_virtual_address(),
        );
        self.command_list.set_graphics_root_shader_resource_view(
            3,
            self.triangle_indices_buffer
                .as_ref()
                .expect("No triangle index buffer created")
                .get_gpu_virtual_address(),
        );
        self.command_list.set_graphics_root_shader_resource_view(
            4,
            self.vertex_indices_buffer
                .as_ref()
                .expect("No vertex index buffer created")
                .get_gpu_virtual_address(),
        );

        self.command_list.set_viewports(&[self.viewport_desc]);
        self.command_list.set_scissor_rects(&[self.scissor_desc]);

        self.command_list
            .resource_barrier(&[ResourceBarrier::new_transition(
                &ResourceTransitionBarrier::default()
                    .with_resource(&self.render_targets[self.frame_index])
                    .with_state_before(ResourceStates::Common)
                    .with_state_after(ResourceStates::RenderTarget),
            )]);

        let rtv_handle = self
            .rtv_heap
            .get_cpu_descriptor_handle_for_heap_start()
            .advance(
                self.swapchain.get_current_back_buffer_index(),
                self.rtv_descriptor_handle_size,
            );

        self.command_list.set_render_targets(
            &[rtv_handle],
            false,
            Some(self.dsv_heap.get_cpu_descriptor_handle_for_heap_start()),
        );

        let clear_color: [f32; 4] = [0.9, 0.2, 0.4, 1.0];
        self.command_list
            .clear_render_target_view(rtv_handle, clear_color, &[]);

        // ToDo: DSV

        self.command_list.dispatch_mesh(self.meshlet_count, 1, 1);

        self.command_list
            .resource_barrier(&[ResourceBarrier::new_transition(
                &ResourceTransitionBarrier::default()
                    .with_resource(&self.render_targets[self.frame_index])
                    .with_state_before(ResourceStates::RenderTarget)
                    .with_state_after(ResourceStates::Common),
            )]);

        self.command_list
            .close()
            .expect("Cannot close command list");
    }

    fn draw(&mut self) {
        trace!("frame index: {}", self.frame_index);

        let last_fence_value = self.fence_values[self.frame_index];
        trace!("fence value: {}", last_fence_value);

        let fence_completed_value = self.fence.get_completed_value();
        trace!("fence completed value: {}", fence_completed_value);

        if fence_completed_value < last_fence_value {
            trace!("waiting on fence");
            self.fence
                .set_event_on_completion(last_fence_value, &self.fence_event)
                .expect("Cannot set event on fence");

            self.fence_event.wait(None);
        }

        self.populate_command_list(self.frame_index);

        self.command_queue
            .execute_command_lists(std::slice::from_ref(&self.command_list));

        self.swapchain
            .present(1, PresentFlags::None)
            .expect("Cannot present");

        self.fence_values[self.frame_index] = last_fence_value + 1;
        trace!(
            "signaling fence with value: {}",
            self.fence_values[self.frame_index]
        );

        self.command_queue
            .signal(&self.fence, self.fence_values[self.frame_index])
            .expect("Cannot signal fence");

        self.frame_index = (self.frame_index + 1) % FRAMES_IN_FLIGHT as usize;

        trace!(
            "setting fence value {} for frame {}",
            last_fence_value + 1,
            self.frame_index
        );
        self.fence_values[self.frame_index] = last_fence_value + 1;

        self.frame_count += 1;
    }

    fn flush_gpu(&mut self) {
        let fence_value = self.fence.get_completed_value() + 1;

        self.command_queue
            .signal(&self.fence, fence_value)
            .expect("Cannot signal fence");

        if self.fence.get_completed_value() < fence_value {
            self.fence
                .set_event_on_completion(fence_value, &self.fence_event)
                .expect("Cannot set event on fence");
            self.fence_event.wait(None);
        }
    }

    fn setup_dsv(&mut self) {
        //let _debug_printer = make_debug_printer!(&self.info_queue);

        let depth_stencil_desc = DepthStencilViewDesc::default()
            .new_texture_2d(Tex2DDsv::default().with_mip_slice(0))
            .with_format(Format::D32Float)
            .with_flags(DsvFlags::None);

        let depth_stencil = self
            .device
            .create_committed_resource(
                &HeapProperties::default().with_heap_type(HeapType::Default),
                HeapFlags::None,
                &ResourceDesc::default()
                    .with_dimension(ResourceDimension::Texture2D)
                    .with_width(WINDOW_WIDTH.into())
                    .with_height(WINDOW_HEIGHT)
                    .with_format(Format::D32Float)
                    .with_flags(
                        ResourceFlags::AllowDepthStencil | ResourceFlags::DenyShaderResource,
                    ),
                ResourceStates::DepthWrite,
                Some(
                    &ClearValue::default()
                        .with_format(Format::D32Float)
                        .with_depth_stencil(
                            &DepthStencilValue::default().with_depth(1.).with_stencil(0),
                        ),
                ),
            )
            .expect("Cannot create depth stencil resource");

        depth_stencil
            .set_name("DepthStencil")
            .expect("Cannot set name on depth stencil");

        self.device.create_depth_stencil_view(
            &depth_stencil,
            &depth_stencil_desc,
            self.dsv_heap.get_cpu_descriptor_handle_for_heap_start(),
        );

        self.depth_stencil = Some(depth_stencil);
    }

    fn setup_constant_buffer(&mut self) {
        let constant_buffer = self
            .device
            .create_committed_resource(
                &HeapProperties::default().with_heap_type(HeapType::Upload),
                HeapFlags::None,
                &ResourceDesc::default()
                    .with_dimension(ResourceDimension::Buffer)
                    .with_width(size_of::<MeshletConstantBuffer>() as u64)
                    .with_layout(TextureLayout::RowMajor),
                ResourceStates::GenericRead,
                None,
            )
            .expect("Cannot create cbuffer staging buffer");

        let constant_buffer_ptr = constant_buffer
            .map(0, None)
            .expect("Cannot map cbv staging buffer");

        let camera = Camera::default();
        let world = Mat4::identity();
        let view = make_view_matrix(camera.position, camera.look_at);
        let proj = make_projection_matrix(&camera);

        eprintln!("view {:?}", &(view));
        eprintln!("wvp {:?}", &(proj * view * world));
        let cb_data = MeshletConstantBuffer {
            mvp: proj * view * world,
            padding: [0.; 48],
        };

        unsafe {
            copy_nonoverlapping(
                &cb_data,
                constant_buffer_ptr as *mut MeshletConstantBuffer,
                1,
            );
        }

        self.constant_buffer_ptr = constant_buffer_ptr;
        self.constant_buffer = Some(constant_buffer);
    }
}

impl Drop for HelloMeshShadersSample {
    fn drop(&mut self) {
        //self.debug_device
        //    .report_live_device_objects()
        //    .expect("Device cannot report live objects");
    }
}

fn create_pipeline_state(
    root_signature: &RootSignature,
    mesh_shader: Vec<u8>,
    pixel_shader: Vec<u8>,
    device: &Device,
) -> PipelineState {
    let ms_bytecode = ShaderBytecode::new(&mesh_shader);
    let ps_bytecode = ShaderBytecode::new(&pixel_shader);

    let pso_subobjects_desc = MeshShaderPipelineStateDesc::default()
        .with_root_signature(root_signature)
        .with_ms_bytecode(&ms_bytecode)
        .with_ps_bytecode(&ps_bytecode)
        .with_rasterizer_state(RasterizerDesc::default().with_depth_clip_enable(false))
        .with_blend_state(BlendDesc::default())
        .with_depth_stencil_state(DepthStencilDesc::default().with_depth_enable(false))
        .with_primitive_topology_type(PrimitiveTopologyType::Triangle)
        .with_rtv_formats(&[Format::R8G8B8A8Unorm]);

    let pso_desc = PipelineStateStreamDesc::default()
        .with_pipeline_state_subobject_stream(pso_subobjects_desc.as_byte_stream());

    device
        .create_pipeline_state(&pso_desc)
        .expect("Cannot create PSO")
}

fn create_shaders() -> (Vec<u8>, Vec<u8>) {
    let mesh_shader = compile_shader(
        "MeshShader",
        &std::fs::read_to_string("assets/mesh_shaders_example_ms.hlsl")
            .expect("Cannot open mesh shader file"),
        "main",
        "ms_6_5",
        &[],
        &[],
    )
    .expect("Cannot compile vertex shader");

    let pixel_shader = compile_shader(
        "PixelShader",
        &std::fs::read_to_string("assets/mesh_shaders_example_ps.hlsl")
            .expect("Cannot open pixel shader file"),
        "main",
        "ps_6_5",
        &[],
        &[],
    )
    .expect("Cannot compile pixel shader");
    (mesh_shader, pixel_shader)
}

fn setup_root_signature(device: &Device, mesh_shader_bytecode: &[u8]) -> RootSignature {
    let root_signature = device
        .create_root_signature(0, &ShaderBytecode::new(mesh_shader_bytecode))
        .expect("Cannot create root signature");
    root_signature
}

fn setup_heaps(
    device: &Device,
    swapchain: &Swapchain,
    rtv_descriptor_handle_size: ByteCount,
) -> (Vec<Resource>, DescriptorHeap, DescriptorHeap) {
    let rtv_heap = device
        .create_descriptor_heap(
            &DescriptorHeapDesc::default()
                .with_heap_type(DescriptorHeapType::Rtv)
                .with_num_descriptors(FRAMES_IN_FLIGHT),
        )
        .expect("Cannot create RTV heap");
    rtv_heap
        .set_name("RTV heap")
        .expect("Cannot set RTV heap name");

    let dsv_heap = device
        .create_descriptor_heap(
            &DescriptorHeapDesc::default()
                .with_heap_type(DescriptorHeapType::Dsv)
                .with_num_descriptors(1),
        )
        .expect("Cannot create RTV heap");
    dsv_heap
        .set_name("DSV heap")
        .expect("Cannot set DSV heap name");

    let mut rtv_handle = rtv_heap.get_cpu_descriptor_handle_for_heap_start();

    let mut render_targets = vec![];
    for frame_idx in 0..FRAMES_IN_FLIGHT {
        let render_target = swapchain
            .get_buffer(frame_idx)
            .expect("cannot get buffer from swapchain");

        device.create_render_target_view(&render_target, rtv_handle);
        render_targets.push(render_target);

        rtv_handle = rtv_handle.advance(1, rtv_descriptor_handle_size);
    }

    (render_targets, rtv_heap, dsv_heap)
}

fn create_swapchain(
    factory: Factory,
    command_queue: &CommandQueue,
    hwnd: *mut std::ffi::c_void,
) -> Swapchain {
    let swapchain_desc = SwapChainDesc::default()
        .with_width(WINDOW_WIDTH)
        .with_height(WINDOW_HEIGHT)
        .with_buffer_count(FRAMES_IN_FLIGHT);
    let swapchain = factory
        .create_swapchain(command_queue, hwnd as *mut HWND__, &swapchain_desc)
        .expect("Cannot create swapchain");
    factory
        .make_window_association(hwnd, MakeWindowAssociationFlags::NoAltEnter)
        .expect("Cannot make window association");
    swapchain
}

fn create_device(factory: &Factory) -> Device {
    if USE_WARP_ADAPTER {
        let warp_adapter = factory
            .enum_warp_adapter()
            .expect("Cannot enum warp adapter");
        return Device::new(&warp_adapter).expect("Cannot create device on WARP adapter");
    }

    let hw_adapter = factory
        .enum_adapters_by_gpu_preference(GpuPreference::HighPerformance)
        .expect("Cannot enumerate adapters")
        .remove(0);
    Device::new(&hw_adapter).expect("Cannot create device")
}

fn insert_unique(coll: &mut Vec<u32>, value: u32) -> u32 {
    if let Some(pos) = coll.iter().position(|&item| item == value) {
        pos as u32
    } else {
        coll.push(value);
        (coll.len() - 1) as u32
    }
}

fn load_mesh() -> (Vec<Vertex>, Vec<Meshlet>, Vec<u32>, Vec<u32>) {
    let (models, _materials) = tobj::load_obj(PATH_TO_OBJ_MODEL, &tobj::GPU_LOAD_OPTIONS)
        .expect("Cannot load mesh from file");

    let mut vertices = vec![];
    let mut triangle_indices: Vec<u32> = vec![];
    let mut vertex_indices: Vec<u32> = vec![];
    let mut meshlets = vec![];

    for (i, m) in models.iter().enumerate() {
        let mesh = &m.mesh;

        // for idx in &mesh.indices  {
        //     trace!("{}, ", idx);
        // }

        for vtx_begin in 0..mesh.positions.len() / 3 {
            vertices.push(Vertex {
                position: Vector3::new(
                    mesh.positions[vtx_begin * 3],
                    mesh.positions[vtx_begin * 3 + 1],
                    mesh.positions[vtx_begin * 3 + 2],
                ),
            });
        }

        trace!(
            "model[{}].vertices: {}, num_face_indices: {}, indices: {}; vertices.len(): {}",
            i,
            mesh.positions.len(),
            mesh.indices.len(),
            mesh.indices.len(),
            vertices.len()
        );

        assert!(mesh.indices.len() % 3 == 0);
        assert!(mesh.positions.len() % 3 == 0);

        let mut current_vertices = vec![];
        let mut current_triangles: Vec<u32> = vec![];

        // let test_indices: [u32; 30] = [
        //     15, 10, 12, 12, 3, 15, 7, 20, 8, 20, 19, 16, 2, 5, 2, 11, 14, 14,
        //     10, 18, 8, 20, 16, 7, 20, 6, 8, 5, 8, 17,
        // ];

        // for tri_begin in (0..test_indices.len()).step_by(3) {
        for tri_begin in (0..mesh.indices.len()).step_by(3) {
            // trace!("Index of triangle start index: {}", tri_begin);

            let current_triangle = [
                // test_indices[tri_begin],
                // test_indices[tri_begin + 1],
                // test_indices[tri_begin + 2],
                mesh.indices[tri_begin],
                mesh.indices[tri_begin + 1],
                mesh.indices[tri_begin + 2],
            ];

            // trace!("Current triangle: {:?}", &current_triangle);

            let current_vertex_count = current_vertices.len();
            let vertex_count_after_add = current_vertex_count
                + current_triangle
                    .iter()
                    .map(|vert| {
                        if current_vertices.contains(vert) {
                            0
                        } else {
                            1
                        }
                    })
                    .sum::<usize>();

            // trace!(
            //     "Current vertex count: {}, vertex count after add: {}",
            //     current_vertex_count,
            //     vertex_count_after_add
            // );

            if vertex_count_after_add > Meshlet::MAX_VERTEX_COUNT
                || current_triangles.len() / 3 + 1 > Meshlet::MAX_TRIANGLE_COUNT
            {
                // trace!("Creating new meshlet");

                meshlets.push(Meshlet {
                    triangle_count: (current_triangles.len() / 3) as u32,
                    triangles_offset: triangle_indices.len() as u32,
                    vertex_count: current_vertices.len() as u32,
                    vertices_offset: vertex_indices.len() as u32,
                });

                triangle_indices.extend(current_triangles.iter());
                vertex_indices.extend(current_vertices.iter());

                current_vertices.clear();
                current_triangles.clear();
            }

            for v_idx in &current_triangle {
                current_triangles.push(insert_unique(&mut current_vertices, *v_idx))
            }
        }

        if !current_triangles.is_empty() {
            meshlets.push(Meshlet {
                triangle_count: (current_triangles.len() / 3) as u32,
                triangles_offset: triangle_indices.len() as u32,
                vertex_count: current_vertices.len() as u32,
                vertices_offset: vertex_indices.len() as u32,
            });

            triangle_indices.extend(current_triangles.iter());
            vertex_indices.extend(current_vertices.iter());
        }
    }

    // println!("triangle indices:");
    // for idx in &triangle_indices {
    //     println!("{:?}, ", idx);
    // }

    // println!("vertex indices:");
    // for idx in &vertex_indices {
    //     println!("{:?}, ", idx);
    // }

    // println!("meshlets:");
    // for m in &meshlets {
    //     println!("{:?}", m);
    // }

    // std::process::exit(0);

    (vertices, meshlets, triangle_indices, vertex_indices)
}

#[derive(Debug, Copy, Clone)]
pub struct Radians(pub f32);

#[derive(Debug, Copy, Clone)]
pub struct Degrees(pub f32);

#[derive(Debug, Copy, Clone)]
pub struct Camera {
    pub near: f32,
    pub far: f32,
    pub fov: Degrees,
    pub aspect: f32,
    pub position: Vec3,
    pub look_at: Vec3,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            near: 0.01,
            far: 100.0,
            fov: Degrees(45.),
            aspect: WINDOW_WIDTH as f32 / WINDOW_HEIGHT as f32,
            // position: Vec3::new(0., 0.5, -1.),
            position: Vec3::new(0., 0., -200.),
            look_at: Vec3::new(0., 0., 10.),
        }
    }
}

fn make_projection_matrix(camera: &Camera) -> Mat4 {
    use cgmath::prelude::*;

    Matrix4::from_cols(
        Vector4 {
            x: 1. / (camera.aspect * cgmath::Deg(camera.fov.0 / 2.).tan()),
            y: 0.,
            z: 0.,
            w: 0.,
        },
        Vector4 {
            x: 0.,
            y: 1. / cgmath::Deg(camera.fov.0 / 2.).tan(),
            z: 0.,
            w: 0.,
        },
        Vector4 {
            x: 0.,
            y: 0.,
            z: camera.far / (camera.far - camera.near),
            w: 1.,
        },
        Vector4 {
            x: 0.,
            y: 0.,
            z: -camera.near * camera.far / (camera.far - camera.near),
            w: 0.,
        },
    )
}

fn make_view_matrix(camera_pos: Vec3, look_at: Vec3) -> Mat4 {
    let cam_k = (look_at - camera_pos).normalize();
    let wrld_up = Vector3::new(0., 1., 0.);
    let cam_i = wrld_up.cross(cam_k).normalize();
    let cam_j = cam_k.cross(cam_i);

    let orientation = Matrix4::from_cols(
        cam_i.extend(0.),
        cam_j.extend(0.),
        cam_k.extend(0.),
        Vector4::new(0., 0., 0., 1.),
    );
    // trace!("orientation matrix: {:?}", &orientation);

    let translation = Matrix4::from_cols(
        Vector4::new(1., 0., 0., 0.),
        Vector4::new(0., 1., 0., 0.),
        Vector4::new(0., 0., 1., 0.),
        Vector4::new(camera_pos[0], camera_pos[1], camera_pos[2], 1.),
    );

    let result = translation * orientation;
    result.invert().expect("No matrix inverse")
}

fn main() {
    let command_args = clap::App::new("HelloMeshShadersSample")
        .arg(
            clap::Arg::with_name("breakonerr")
                .short("b")
                .takes_value(false)
                .help("Break on validation errors"),
        )
        .arg(
            clap::Arg::with_name("v")
                .short("v")
                .multiple(true)
                .help("Verbosity level"),
        )
        .get_matches();

    let log_level = match command_args.occurrences_of("v") {
        0 => log::Level::Info,
        1 => log::Level::Debug,
        _ => log::Level::Trace,
    };

    simple_logger::init_with_level(log_level).unwrap();

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .build(&event_loop)
        .expect("Cannot create window");
    window.set_inner_size(winit::dpi::LogicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT));
    // ToDo: command line for break_on_warn
    let mut sample = HelloMeshShadersSample::new(
        window.hwnd(), /* , command_args.is_present("breakonerr")*/
    );

    let mut fps_counter = FPSCounter::new(std::time::Duration::from_millis(1000));

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Poll;
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            Event::MainEventsCleared => {
                window.request_redraw();
            }
            Event::RedrawRequested(_) => {
                // Redraw the application.
                //
                // It's preferrable to render in this event rather than in MainEventsCleared, since
                // rendering in here allows the program to gracefully handle redraws requested
                // by the OS.

                sample.draw();

                fps_counter.end_frame();
                window.set_title(format_performance_message(&fps_counter).as_str());
            }
            _ => (),
        }
    });
}

fn format_performance_message(fps_counter: &FPSCounter) -> String {
    format!(
        "Arrow example ({} FPS, {:.4} ms)",
        fps_counter.current_fps(),
        (fps_counter.average_render_time() as f64 * 0.000001)
    )
}
